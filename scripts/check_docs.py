#!/usr/bin/env python3
"""Check local documentation links and current release version references.

Adapted from kohdalab-trkr 066f8968; MIT notice in third_party/kohdalab-trkr."""
from __future__ import annotations
import re
import sys
import tomllib
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
IGNORED = {'.git', 'target', 'build', 'dist', '__pycache__', 'vendor', 'third-party-licenses'}
LINK = re.compile(r'\]\((<[^>]+>|[^\s)]+)(?:\s+[^)]*)?\)')
HTML_SOURCE = re.compile(r'\b(?:src|href)=[\"\x27]([^\"\x27]+)[\"\x27]')


def documents() -> list[Path]:
    paths = set(ROOT.glob('*.md'))
    for folder in ['docs', '.github', 'bindings', 'bridge', 'servers', 'packaging', 'crates', 'assets', 'notebook']:
        base = ROOT / folder
        if base.is_dir():
            paths.update(base.rglob('*.md'))
    return sorted(path for path in paths if not any(part in IGNORED for part in path.relative_to(ROOT).parts)
                  and 'releases' not in path.relative_to(ROOT).parts)


def toml_version(root: Path, path: str, *keys: str) -> str | None:
    data = tomllib.loads((root / path).read_text(encoding='utf-8'))
    for key in keys:
        data = data.get(key) if isinstance(data, dict) else None
    return data if isinstance(data, str) else None


def markdown_version(text: str, pattern: str) -> str | None:
    match = re.search(pattern, text, re.MULTILINE)
    return match[1] if match else None


def downloads_versions(text: str) -> list[tuple[str, str | None]]:
    section = re.search(r'^## Downloads\b[^\n]*\n(.*?)(?=^#{1,2} |\Z)', text, re.M | re.S)
    if section is None:
        return [('README.md Downloads table', None)]
    versions = []
    for row in section[1].splitlines():
        if not row.startswith('|'):
            continue
        columns = row.split('|')
        edition = markdown_version(columns[1].strip(), r'^v?(\d\S*)')
        if edition is None:
            continue
        release = columns[2] if len(columns) > 2 else ''
        versions.extend([
            ('README.md Downloads edition', edition),
            ('README.md Downloads release label', markdown_version(release, r'\[Release v?([^\s\]]+)\]')),
            ('README.md Downloads release link', markdown_version(release, r'/releases/tag/v?([^\s)]+)\)')),
        ])
    return versions or [('README.md Downloads table', None)]


def version_errors(root: Path) -> list[str]:
    expected = toml_version(root, 'Cargo.toml', 'workspace', 'package', 'version')
    if not expected:
        return ['Cargo.toml: missing [workspace.package] version']

    def read(path: str) -> str:
        return (root / path).read_text(encoding='utf-8')

    references = [
        ('START-HERE.md title', markdown_version(read('START-HERE.md'), r'^# KohdaLab PLE v?(\S+)')),
        ('RELEASE-NOTES.md title', markdown_version(read('RELEASE-NOTES.md'), r'^# KohdaLab PLE v?(\S+)')),
        *downloads_versions(read('README.md')),
        ('bindings/python/pyproject.toml [project] version',
         toml_version(root, 'bindings/python/pyproject.toml', 'project', 'version')),
        ('CHANGELOG.md first version heading',
         markdown_version(read('CHANGELOG.md'), r'^## v?(\d\S*)')),
    ]
    return [
        f'{source}: expected {expected} from Cargo.toml, found {actual or "no version"}'
        for source, actual in references if actual != expected
    ]


def main() -> int:
    errors = []
    checked = 0
    paths = documents()
    for path in paths:
        text = re.sub(r'^(```|~~~).*?^\1[^\n]*$', '', path.read_text(encoding='utf-8'), flags=re.M | re.S)
        targets = [match.group(1).strip('<>') for match in LINK.finditer(text)]
        targets += HTML_SOURCE.findall(text)
        for target in targets:
            parsed = urlsplit(target)
            if parsed.scheme or parsed.netloc or not parsed.path or parsed.path.startswith('/'):
                continue
            local = (path.parent / unquote(parsed.path)).resolve()
            checked += 1
            if not local.is_relative_to(ROOT.resolve()) or not local.exists():
                errors.append(f'{path.relative_to(ROOT)}: {target}')
    if errors:
        print('\n'.join(errors), file=sys.stderr)
        print(f'FAIL: {len(errors)} broken local links', file=sys.stderr)
    try:
        mismatches = version_errors(ROOT)
    except (OSError, tomllib.TOMLDecodeError) as error:
        mismatches = [str(error)]
    if mismatches:
        print('\n'.join(mismatches), file=sys.stderr)
        print(f'FAIL: {len(mismatches)} release version mismatches', file=sys.stderr)
    if errors or mismatches:
        return 1
    print(f'PASS: {len(paths)} documents, {checked} local links, release versions agree')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
