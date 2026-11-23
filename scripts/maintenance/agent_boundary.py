#!/usr/bin/env python3
"""Linux offline agent boundary. No online mode is enabled or implied."""
import argparse
import base64
import difflib
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import socket
import socketserver
import subprocess
import sys
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = Path(__file__).resolve()
MARKDOWN = ('.md', '.mdx', '.markdown')
LIMIT = 8 * 1024 * 1024


def digest(data):
    return hashlib.sha256(data).hexdigest()


def relative(name):
    p = PurePosixPath(name)
    if not name or p.is_absolute() or '..' in p.parts or str(p) != name:
        raise ValueError('Expected normalized repository-relative path')
    return p


def source_path(name):
    p = relative(name)
    if any(part.startswith('.') for part in p.parts):
        raise ValueError('Hidden configuration paths are protected')
    if p.suffix.lower() in MARKDOWN:
        raise ValueError('Public Markdown requires a documentation proposal')
    if p.parts[0] not in ('crates', 'tests', 'examples', 'benches') and name not in ('Cargo.toml', 'Cargo.lock'):
        raise ValueError('Path is outside the source editor capability')
    return p


def parent_fd(root, name, create=False):
    """Walk from a trusted root descriptor; never traverse links."""
    parts = relative(name).parts
    fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for part in parts[:-1]:
            if create:
                try:
                    os.mkdir(part, dir_fd=fd)
                except FileExistsError:
                    pass
            nxt = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = nxt
        return fd, parts[-1]
    except BaseException:
        os.close(fd)
        raise


def read_at(fd, name):
    try:
        f = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
    except FileNotFoundError:
        return None
    import stat
    with os.fdopen(f, 'rb') as stream:
        status = os.fstat(stream.fileno())
        if not stat.S_ISREG(status.st_mode) or status.st_nlink != 1:
            raise ValueError('Only regular, singly linked files are supported')
        data = stream.read(LIMIT + 1)
        if len(data) > LIMIT:
            raise ValueError('File exceeds editor limit')
        return data


def write_at(fd, name, data):
    tmp = '.agent-edit-' + os.urandom(16).hex()
    mode = 0o644
    try:
        mode = os.stat(name, dir_fd=fd, follow_symlinks=False).st_mode & 0o777
    except FileNotFoundError:
        pass
    f = os.open(tmp, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, mode, dir_fd=fd)
    try:
        with os.fdopen(f, 'wb') as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.rename(tmp, name, src_dir_fd=fd, dst_dir_fd=fd)
        os.fsync(fd)
    finally:
        try:
            os.unlink(tmp, dir_fd=fd)
        except FileNotFoundError:
            pass


def edit(root, request):
    name = str(source_path(request['path']))
    fd, leaf = parent_fd(root, name, create=request['op'] == 'put')
    try:
        old = read_at(fd, leaf)
        actual = digest(old) if old is not None else None
        if request['op'] == 'read':
            return {'hash': actual, 'data': base64.b64encode(old or b'').decode()}
        if request.get('base') != actual:
            raise ValueError('Source changed; read and retry')
        if request['op'] == 'put':
            data = base64.b64decode(request['data'], validate=True)
            if len(data) > LIMIT:
                raise ValueError('File exceeds editor limit')
            write_at(fd, leaf, data)
        elif request['op'] == 'delete':
            os.unlink(leaf, dir_fd=fd)
        else:
            raise ValueError('Unsupported source operation')
        return {'ok': True}
    finally:
        os.close(fd)


def client(request):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.connect('/boundary/source.sock')
        connection.sendall(json.dumps(request).encode() + b'\n')
        response = json.loads(connection.makefile('rb').readline(LIMIT * 2))
    if 'error' in response:
        raise ValueError(response['error'])
    return response


def propose(target, candidate, justification):
    relative(target)
    if Path(target).suffix.lower() not in MARKDOWN or target.startswith('.'):
        raise ValueError('Proposal must target public Markdown')
    old = (ROOT / target).read_bytes() if (ROOT / target).exists() else None
    new = Path(candidate).read_bytes()
    patch = ''.join(difflib.unified_diff(
        (old or b'').decode('utf-8').splitlines(keepends=True),
        new.decode('utf-8').splitlines(keepends=True),
        fromfile=target, tofile=target))
    proposal = {'target': target, 'base': digest(old) if old is not None else None,
                'data': base64.b64encode(new).decode(), 'diff': patch,
                'justification': justification}
    data = json.dumps(proposal, sort_keys=True, indent=2).encode() + b'\n'
    folder = ROOT / '.private/doc-proposals' / digest(data)
    folder.mkdir(parents=True, exist_ok=False)
    (folder / 'proposal.json').write_bytes(data)
    print(folder)


def review(path):
    if os.environ.get('AGENT_BOUNDARY') or not sys.stdin.isatty() or not sys.stdout.isatty():
        raise ValueError('Review requires a separate trusted host terminal')
    data = Path(path).read_bytes()  # immutable in-memory snapshot
    proposal = json.loads(data)
    target = str(relative(proposal['target']))
    if Path(target).suffix.lower() not in MARKDOWN or target.startswith('.'):
        raise ValueError('Only public Markdown can be approved')
    new = base64.b64decode(proposal['data'], validate=True)
    fd, leaf = parent_fd(ROOT, target, create=False)
    try:
        old = read_at(fd, leaf)
        if (digest(old) if old is not None else None) != proposal['base']:
            raise ValueError('Stale proposal base')
        patch = ''.join(difflib.unified_diff((old or b'').decode().splitlines(True),
                       new.decode().splitlines(True), fromfile=target, tofile=target))
        if patch != proposal['diff']:
            raise ValueError('Diff does not match proposed bytes')
        print('Target:', target)
        print('Justification:', ascii(proposal['justification']))
        print('Patch (escaped to prevent terminal control sequences):')
        for line in patch.splitlines():
            print(ascii(line))
        approval = digest(data)
        if input('Type the full proposal digest to apply: ' + approval + '\n') != approval:
            raise ValueError('Not approved')
        if Path(path).read_bytes() != data:
            raise ValueError('Proposal changed during review')
        if read_at(fd, leaf) != old:
            raise ValueError('Target changed during review')
        receipts = ROOT / '.git/documentation-approvals'
        receipts.mkdir(mode=0o700, exist_ok=True)
        (receipts / (approval + '.json')).write_text(json.dumps({
            'target': target, 'before': proposal['base'], 'after': digest(new),
            'original': base64.b64encode(old or b'').decode(), 'proposal': approval}))
        write_at(fd, leaf, new)
        policy_path = ROOT / 'scripts/maintenance/public-documentation.json'
        policy = json.loads(policy_path.read_text())
        if target not in policy['public_markdown']:
            policy['public_markdown'] = sorted(policy['public_markdown'] + [target])
            policy_path.write_text(json.dumps(policy, indent=2) + '\n')
    finally:
        os.close(fd)
    print('Approved bytes applied locally. Nothing staged or published.')


def launch(command):
    if os.environ.get('AGENT_BOUNDARY'):
        raise ValueError('Launch only from the trusted host')
    if sys.platform != 'linux' or not shutil.which('bwrap'):
        raise ValueError('Bubblewrap on Linux is required; no unrestricted fallback')
    private = ROOT / '.private'
    private.mkdir(mode=0o700, exist_ok=True)
    # Writable aliases must not expose the inode of a read-only document.
    for p in private.rglob('*'):
        if p.is_file() and p.stat().st_nlink > 1:
            raise ValueError('Hardlink in private storage: ' + str(p))
    with tempfile.TemporaryDirectory(prefix='prime-boundary-') as temp:
        state = Path(temp)
        home = state / 'home'
        home.mkdir()
        metadata = state / 'git'
        metadata.mkdir()
        (metadata / 'config').write_text('[core]\nrepositoryformatversion = 0\nbare = false\nhooksPath = /dev/null\n')
        for name in ['HEAD', 'index', 'packed-refs', 'shallow']:
            if (ROOT / '.git' / name).is_file():
                shutil.copyfile(ROOT / '.git' / name, metadata / name)
        if (ROOT / '.git/refs').exists():
            shutil.copytree(ROOT / '.git/refs', metadata / 'refs')
        (metadata / 'objects').mkdir()
        build = state / 'build' 
        build.mkdir()
        class Handler(socketserver.StreamRequestHandler):
            def handle(self):
                try:
                    req = json.loads(self.rfile.readline(LIMIT * 2))
                    answer = edit(ROOT, req)
                except Exception as error:
                    answer = {'error': str(error)}
                self.wfile.write(json.dumps(answer).encode() + b'\n')
        server = socketserver.UnixStreamServer(str(state / 'source.sock'), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        args = ['bwrap', '--unshare-all', '--unshare-user', '--disable-userns',
                '--assert-userns-disabled', '--die-with-parent', '--new-session',
                '--cap-drop', 'ALL', '--clearenv', '--ro-bind', '/usr', '/usr',
                '--symlink', 'usr/lib', '/lib', '--symlink', 'usr/lib64', '/lib64',
                '--symlink', 'usr/bin', '/bin', '--proc', '/proc', '--dev', '/dev',
                '--tmpfs', '/tmp', '--dir', '/etc', '--ro-bind', '/etc/ssl', '/etc/ssl',
                '--ro-bind', str(ROOT), str(ROOT), '--ro-bind', str(metadata), str(ROOT / '.git'),
                '--ro-bind', str(ROOT / '.git/objects'), str(ROOT / '.git/objects'),
                '--bind', str(private), str(private), '--bind', str(home), '/home/agent',
                '--dir', '/boundary', '--ro-bind', str(state / 'source.sock'), '/boundary/source.sock',
                '--bind', str(build), str(ROOT.parent / '.cache/prime-emulator-target'),
                '--setenv', 'HOME', '/home/agent', '--setenv', 'AGENT_BOUNDARY', 'offline',
                '--setenv', 'PATH', '/usr/bin:/bin', '--setenv', 'TERM', 'xterm-256color',
                '--setenv', 'CARGO_TARGET_DIR', str(ROOT.parent / '.cache/prime-emulator-target'),
                '--chdir', str(ROOT)]
        # Installed toolchains and dependency caches are read-only; no host config/credentials.
        for folder in ['.rustup', '.cargo/registry', '.cargo/bin']:
            src = Path.home() / folder
            if src.exists():
                args += ['--ro-bind', str(src), '/home/agent/' + folder]
        args += ['--setenv', 'PATH', '/home/agent/.cargo/bin:/usr/bin:/bin']
        npm = Path.home() / '.local/share/npm'
        if npm.exists():
            args += ['--ro-bind', str(npm), str(npm)]
        print('OFFLINE: no provider authentication, GitHub credentials or network exposed.', flush=True)
        try:
            return subprocess.call(args + ['--'] + command, close_fds=True)
        finally:
            server.shutdown()
            server.server_close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    run = sub.add_parser('run', help='Run an offline development command inside the boundary')
    run.add_argument('command', nargs=argparse.REMAINDER)
    for tool in ['codex', 'claude', 'gemini']:
        sub.add_parser(tool, help='Fail closed until restricted provider connectivity is verified')
    source = sub.add_parser('source')
    source.add_argument('operation', choices=['read', 'put', 'delete'])
    source.add_argument('path')
    source.add_argument('--base', default=None)
    proposal = sub.add_parser('propose')
    proposal.add_argument('target')
    proposal.add_argument('candidate')
    proposal.add_argument('--why', required=True)
    trusted = sub.add_parser('review-apply')
    trusted.add_argument('proposal')
    args = parser.parse_args()
    if args.action == 'run':
        return launch(args.command)
    if args.action in ('codex', 'claude', 'gemini'):
        parser.error('Online launcher disabled: restricted authenticated provider connectivity is not verified. Use run for offline tools. No unrestricted fallback.')
    if args.action == 'source':
        request = {'op': args.operation, 'path': args.path, 'base': args.base}
        if args.operation == 'put':
            request['data'] = base64.b64encode(sys.stdin.buffer.read(LIMIT + 1)).decode()
        print(json.dumps(client(request)))
    elif args.action == 'propose':
        propose(args.target, args.candidate, args.why)
    else:
        review(args.proposal)
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (ValueError, OSError, KeyError) as error:
        print('Boundary refused: ' + str(error), file=sys.stderr)
        raise SystemExit(1)
