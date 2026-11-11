"""Regression tests for the documentation release version check."""
import tempfile
import unittest
from pathlib import Path

from scripts.check_docs import version_errors


class DocsVersionTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.write('Cargo.toml', '[workspace.package]\nversion = "0.3.11"\n')
        self.write('START-HERE.md', '# KohdaLab PLE v0.3.11 — Windows\n')
        self.write('RELEASE-NOTES.md', '# KohdaLab PLE 0.3.11 preview\n')
        self.write('README.md', """# KohdaLab PLE

## Downloads / ダウンロード

| Edition | Package |
| --- | --- |
| v0.3.11 preview | [Release v0.3.11](https://github.com/woolen-yarn/kohdalab-ple/releases/tag/v0.3.11) |

## Archive
| v0.3.10 preview | old |
""")
        self.write('bindings/python/pyproject.toml', '[project]\nversion = "0.3.11"\n')
        self.write('CHANGELOG.md', '## Unreleased / 未リリース\n\n## 0.3.11 — current\n\n## 0.3.10\n')

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding='utf-8')

    def assert_drift(self, name, source, old='0.3.11', new='0.3.8'):
        path = self.root / name
        text = path.read_text(encoding='utf-8')
        self.assertIn(old, text)
        self.write(name, text.replace(old, new, 1))
        self.assertEqual(
            version_errors(self.root),
            [f'{source}: expected 0.3.11 from Cargo.toml, found 0.3.8'],
        )

    def test_current_versions_agree(self):
        self.assertEqual(version_errors(self.root), [])

    def test_start_here_title_drift(self):
        self.assert_drift('START-HERE.md', 'START-HERE.md title')

    def test_release_notes_title_drift(self):
        self.assert_drift('RELEASE-NOTES.md', 'RELEASE-NOTES.md title')

    def test_downloads_edition_drift(self):
        self.assert_drift('README.md', 'README.md Downloads edition')

    def test_downloads_release_label_drift(self):
        self.assert_drift('README.md', 'README.md Downloads release label',
                          'Release v0.3.11', 'Release v0.3.8')

    def test_downloads_release_link_drift(self):
        self.assert_drift('README.md', 'README.md Downloads release link',
                          '/releases/tag/v0.3.11', '/releases/tag/v0.3.8')

    def test_python_client_version_drift(self):
        self.assert_drift('bindings/python/pyproject.toml',
                          'bindings/python/pyproject.toml [project] version')

    def test_first_changelog_version_drift(self):
        self.assert_drift('CHANGELOG.md', 'CHANGELOG.md first version heading')

    def test_missing_downloads_section(self):
        self.write('README.md', '# KohdaLab PLE\n')
        self.assertEqual(
            version_errors(self.root),
            ['README.md Downloads table: expected 0.3.11 from Cargo.toml, found no version'],
        )
