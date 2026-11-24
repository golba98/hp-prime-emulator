#!/usr/bin/env python3
"""Review public documentation paths and reject private files before publication."""
import argparse
import json
import subprocess
from pathlib import Path

POLICY = 'scripts/maintenance/public-documentation.json'
RESERVED = ('.private/', '.agents/', '.claude/', '.gemini/', 'scratch/', 'artifacts/')


def git(*args):
    return subprocess.check_output(['git', *args])


def check(paths, policy):
    approved = set(policy['public_markdown'])
    errors = []
    for path in sorted(paths):
        if any(path.startswith(prefix) for prefix in RESERVED):
            errors.append(f'Private path cannot be published: {path}')
        elif path.lower().endswith(('.md', '.mdx', '.markdown')) and path not in approved:
            errors.append(f'Public Markdown needs explicit review in {POLICY}: {path}')
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group()
    group.add_argument('--staged', action='store_true')
    group.add_argument('--revision', help='Check a committed tree')
    parser.add_argument('--install-hooks', action='store_true',
                        help='Install local guards; refuse to replace existing hooks')
    parser.add_argument('--review-content', action='store_true',
                        help='Verify local public content against the frozen baseline or human receipts')
    args = parser.parse_args()
    root = Path(git('rev-parse', '--show-toplevel').decode().strip())
    if args.install_hooks:
        hooks = Path(git('rev-parse', '--git-path', 'hooks').decode().strip())
        hooks.mkdir(exist_ok=True)
        bodies = {
            'pre-commit': 'exec python3 scripts/maintenance/check_documentation.py --staged --review-content\n',
            'pre-push': (
                'while read local_ref local_sha remote_ref remote_sha; do\n'
                ' case "$local_sha" in 0000000000000000000000000000000000000000) continue ;; esac\n'
                ' python3 scripts/maintenance/check_documentation.py --revision "$local_sha" --review-content || exit 1\n'
                'done\n'
            ),
        }
        for name, body in bodies.items():
            path = hooks / name
            content = '#!/bin/sh\n# Repository documentation publication guard.\n' + body
            if path.exists() and path.read_text() != content:
                parser.error(f'Existing hook preserved: {path}; integrate the guard manually')
        for name, body in bodies.items():
            path = hooks / name
            path.write_text('#!/bin/sh\n# Repository documentation publication guard.\n' + body)
            path.chmod(0o755)
        print('Local publication hooks installed.')
        return False
    if args.revision:
        paths = git('ls-tree', '-rz', '--name-only', args.revision).decode().split('\0')
        policy = json.loads(git('show', f'{args.revision}:{POLICY}'))
    elif args.staged:
        paths = git('ls-files', '-z').decode().split('\0')
        policy = json.loads(git('show', f':{POLICY}'))
    else:
        paths = git('ls-files', '-z', '--cached', '--others', '--exclude-standard').decode().split('\0')
        paths = [p for p in paths if (root / p).exists()]
        policy = json.loads((root / POLICY).read_text())
    errors = check(set(paths), policy)
    if args.review_content:
        import hashlib
        common = Path(git('rev-parse', '--git-common-dir').decode().strip())
        baseline_path = common / 'documentation-baseline.json'
        if not baseline_path.exists():
            errors.append('Trusted local documentation baseline is unavailable')
        else:
            baseline = json.loads(baseline_path.read_text())
            receipts = []
            for receipt in (common / 'documentation-approvals').glob('*.json'):
                receipts.append(json.loads(receipt.read_text()))
            for path in paths:
                if not path.lower().endswith(('.md', '.mdx', '.markdown')):
                    continue
                if args.revision:
                    data = git('show', f'{args.revision}:{path}')
                elif args.staged:
                    data = git('show', f':{path}')
                else:
                    data = (root / path).read_bytes()
                content_hash = hashlib.sha256(data).hexdigest()
                accepted = baseline.get(path) == content_hash or any(
                    r['target'] == path and r['after'] == content_hash for r in receipts)
                if not accepted:
                    errors.append(f'Public content requires trusted review: {path}')
    for error in errors:
        print(error)
    if not errors:
        print('Documentation publication check passed.')
    return bool(errors)


if __name__ == '__main__':
    raise SystemExit(main())
