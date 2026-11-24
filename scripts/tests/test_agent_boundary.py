"""The editor tests use disposable roots; no real approval or publication."""
import base64
import importlib.util
import tempfile
import sys
from pathlib import Path
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'maintenance/agent_boundary.py'
spec = importlib.util.spec_from_file_location('boundary', SCRIPT)
boundary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boundary)


@unittest.skipUnless(sys.platform == "linux", "Linux descriptor confinement")
class SourceBoundaryTests(unittest.TestCase):
    def test_documentation_and_security_paths_denied(self):
        for name in ['README.md', 'crates/new.MD', 'docs/guide.md',
                     'crates/new.mdx', '../Cargo.toml', '/tmp/file',
                     '.git/config', 'scripts/maintenance/agent_boundary.py']:
            with self.subTest(name=name), self.assertRaises(ValueError):
                boundary.source_path(name)

    def test_source_changes_require_matching_base(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            req = {'op': 'put', 'path': 'crates/demo/src/lib.rs', 'base': None,
                   'data': base64.b64encode(b'pub fn answer() -> u8 { 42 }').decode()}
            self.assertEqual(boundary.edit(root, req), {'ok': True})
            with self.assertRaises(ValueError):
                boundary.edit(root, req)
            current = boundary.edit(root, {'op': 'read', 'path': req['path']})
            boundary.edit(root, {'op': 'delete', 'path': req['path'], 'base': current['hash']})
            self.assertFalse((root / req['path']).exists())

    def test_symlink_and_hardlink_aliases_denied(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'crates').mkdir()
            (root / 'README.md').write_text('public')
            (root / 'crates/alias.rs').symlink_to(root / 'README.md')
            with self.assertRaises(OSError):
                boundary.edit(root, {'op': 'read', 'path': 'crates/alias.rs'})
            import os
            os.link(root / 'README.md', root / 'crates/hard.rs')
            with self.assertRaises(ValueError):
                boundary.edit(root, {'op': 'read', 'path': 'crates/hard.rs'})
            (root / 'crates/alias').symlink_to(root, target_is_directory=True)
            with self.assertRaises(OSError):
                boundary.edit(root, {'op': 'put', 'path': 'crates/alias/file.rs',
                                     'base': None, 'data': ''})


@unittest.skipUnless(sys.platform == "linux", "Linux trusted reviewer")
class ReviewBoundaryTests(unittest.TestCase):
    def test_changed_and_forged_proposals_are_refused(self):
        from unittest.mock import patch
        import json
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / '.git').mkdir()
            (root / 'README.md').write_bytes(b'old\n')
            (root / 'scripts/maintenance').mkdir(parents=True)
            (root / 'scripts/maintenance/public-documentation.json').write_text(
                json.dumps({'public_markdown': ['README.md']}))
            proposal = root / 'proposal.json'
            new = b'new\n'
            obj = {'target': 'README.md', 'base': boundary.digest(b'old\n'),
                   'data': base64.b64encode(new).decode(),
                   'justification': 'test', 'diff': 'forged diff'}
            proposal.write_text(json.dumps(obj))
            with patch.object(boundary, 'ROOT', root), \
                 patch.object(boundary.sys.stdin, 'isatty', return_value=True), \
                 patch.object(boundary.sys.stdout, 'isatty', return_value=True):
                with self.assertRaises(ValueError):
                    boundary.review(proposal)
                obj['diff'] = ''.join(boundary.difflib.unified_diff(
                    ['old\n'], ['new\n'], fromfile='README.md', tofile='README.md'))
                proposal.write_text(json.dumps(obj))
                snapshot = proposal.read_bytes()

                def tamper(_):
                    proposal.write_text('{}')
                    return boundary.digest(snapshot)

                with patch('builtins.input', side_effect=tamper), self.assertRaises(ValueError):
                    boundary.review(proposal)
                self.assertEqual((root / 'README.md').read_bytes(), b'old\n')
                proposal.write_bytes(snapshot)
                with patch('builtins.input', return_value='not approved'), self.assertRaises(ValueError):
                    boundary.review(proposal)
                with patch('builtins.input', return_value=boundary.digest(snapshot)):
                    boundary.review(proposal)
                self.assertEqual((root / 'README.md').read_bytes(), new)


if __name__ == '__main__':
    unittest.main()
