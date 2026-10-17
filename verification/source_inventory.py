"""Check whether the committed workspace source files are present."""

from pathlib import Path
import os
import re
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def main():
    with (ROOT / 'Cargo.toml').open('rb') as source:
        members = tomllib.load(source)['workspace']['members']
    errors = set()
    manifest_files = [ROOT / 'Cargo.toml']
    for member in members:
        path = ROOT / member / 'Cargo.toml'
        if not path.is_file():
            errors.add(f'Missing workspace member: {member}/Cargo.toml')
        else:
            manifest_files.append(path)

    fuzz_manifest = ROOT / 'fuzz/Cargo.toml'
    if fuzz_manifest.is_file():
        manifest_files.append(fuzz_manifest)

    for path in manifest_files:
        with path.open('rb') as source:
            manifest = tomllib.load(source)
        for section in ('dependencies', 'dev-dependencies', 'build-dependencies'):
            for name, dependency in manifest.get(section, {}).items():
                if isinstance(dependency, dict) and 'path' in dependency:
                    required = path.parent / dependency['path'] / 'Cargo.toml'
                    if not required.is_file():
                        errors.add(f'{path.relative_to(ROOT)}: path dependency {name} is missing')
        for target in manifest.get('bin', []):
            if 'path' in target and not (path.parent / target['path']).is_file():
                errors.add(f'{path.relative_to(ROOT)}: binary target {target["name"]} source is missing')

    module_declaration = re.compile(r'^\s*(?:pub(?:\([^\n)]*\))?\s+)?mod\s+(\w+)\s*;', re.MULTILINE)
    for path in (ROOT / 'crates').glob('*/src/**/*.rs'):
        # External modules require a sibling .rs or a directory/mod.rs.
        base = path.parent if path.stem in ('lib', 'main', 'mod') else path.parent / path.stem
        for match in module_declaration.finditer(path.read_text(encoding='utf-8')):
            module = match.group(1)
            if not (base / (module + '.rs')).is_file() and not (base / module / 'mod.rs').is_file():
                errors.add(f'{path.relative_to(ROOT)}: declared module {module} is missing')

    for error in sorted(errors):
        print('Missing source:', error)
    if errors:
        print(f'Source inventory FAILED: {len(errors)} unresolved references.')
        print('Restore the real implementation files before running full Cargo CI.')
    else:
        print('Source inventory passed.')

    summary = os.environ.get('GITHUB_STEP_SUMMARY')
    if summary:
        with open(summary, 'a', encoding='utf-8') as output:
            if errors:
                output.write(f'### Source inventory blocked: {len(errors)} missing references\n\n')
                output.write('Restore the original source implementations before full CI can pass.\n\n')
                for item in sorted(errors):
                    output.write(f'- {item}\n')
            else:
                output.write('### Source inventory complete\n')
    return bool(errors)


if __name__ == '__main__':
    raise SystemExit(main())
