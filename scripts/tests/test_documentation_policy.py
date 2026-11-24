"""Publication guard tests use disposable repositories, never the caller's index."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'maintenance/check_documentation.py'
spec = importlib.util.spec_from_file_location('guard', SCRIPT)
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


class DocumentationPolicyTests(unittest.TestCase):
    def test_reviewed_docs_and_private_paths(self):
        policy = {'public_markdown': ['README.md', 'tests/fixtures/README.md']}
        self.assertFalse(guard.check(['README.md', 'tests/fixtures/README.md'], policy))
        for path in ['docs/new.md', '.private/notes.md', '.agents/output.json']:
            self.assertTrue(guard.check([path], policy))

    def test_staged_policy_and_force_added_notes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env = dict(os.environ)
            for key in ['GIT_INDEX_FILE', 'GIT_DIR', 'GIT_WORK_TREE']:
                env.pop(key, None)

            def git(*args):
                subprocess.run(['git', *args], cwd=root, env=env, check=True,
                               capture_output=True)

            def check():
                return subprocess.run([sys.executable, str(SCRIPT), '--staged'],
                                      cwd=root, env=env, capture_output=True).returncode

            git('init', '-q')
            policy = root / guard.POLICY
            policy.parent.mkdir(parents=True)
            policy.write_text(json.dumps({'public_markdown': ['README.md']}))
            (root / 'README.md').write_text('Public guide')
            git('add', '.')
            self.assertEqual(check(), 0)
            (root / 'new.md').write_text('Needs review')
            git('add', 'new.md')
            policy.write_text(json.dumps({'public_markdown': ['README.md', 'new.md']}))
            self.assertNotEqual(check(), 0)  # unstaged approval cannot authorize staged file
            git('add', guard.POLICY)
            self.assertEqual(check(), 0)
            (root / '.gitignore').write_text('/.private/\n')
            (root / '.private').mkdir()
            (root / '.private/notes.md').write_text('Private')
            git('add', '.')
            self.assertEqual(check(), 0)
            git('add', '-f', '.private/notes.md')
            self.assertNotEqual(check(), 0)


if __name__ == '__main__':
    unittest.main()
