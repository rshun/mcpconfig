"""Publish verified candidate artifacts, or safely resume an interrupted draft."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
from urllib.parse import quote


def digest(data):
    return 'sha256:' + hashlib.sha256(data).hexdigest()


def expected_names(version):
    return {
        f'mcp-hub-v{version}-linux-x64.tar.gz',
        f'mcp-hub-v{version}-windows-x64.zip',
        f'mcp-hub-v{version}-macos-arm64.tar.gz',
        f'MCP.Hub_{version}_x64-setup.exe',
        f'MCP.Hub_{version}_aarch64.dmg',
        'customer-mcp.yaml',
    }


def checksums(files):
    return ''.join(f'{digest(data)[7:]}  {name}\n' for name, data in sorted(files.items())).encode()


def prepare(files, version, release):
    original = files['SHA256SUMS']
    source = {name: data for name, data in files.items() if name != 'SHA256SUMS'}
    if checksums(source) != original:
        raise ValueError('Candidate checksums do not match its files')
    normalized = {name.replace('MCP Hub_', 'MCP.Hub_'): data for name, data in source.items()}
    if len(normalized) != len(source) or set(normalized) != expected_names(version):
        raise ValueError('Unexpected candidate asset set')
    assets = {a['name']: a for a in release['assets']} if release else {}
    if release and not release['draft']:
        raise ValueError('Release is already published; refusing to modify it')
    current = assets.get('SHA256SUMS')
    backup = False
    if 'SHA256SUMS.source' in assets:
        normalized['SHA256SUMS.source'] = original
    elif current and current['digest'] != digest(checksums(normalized)):
        if current['digest'] != digest(original):
            raise ValueError('Existing checksum asset is not the original candidate manifest')
        normalized['SHA256SUMS.source'] = original
        backup = True
    normalized['SHA256SUMS'] = checksums(normalized)
    # Complete preflight before renaming or uploading anything.
    for name, asset in assets.items():
        target = 'SHA256SUMS.source' if backup and name == 'SHA256SUMS' else name
        verify_asset(asset, normalized, target)
    return normalized, backup


def verify_asset(asset, files, name=None):
    name = name or asset['name']
    if name not in files or asset['state'] != 'uploaded':
        raise ValueError(f'Unexpected or incomplete asset: {name}')
    if asset['size'] != len(files[name]) or asset.get('digest') != digest(files[name]):
        raise ValueError(f'Asset content mismatch: {name}')


def api(path, method='GET', body=None, file=None):
    args = ['gh', 'api', '--method', method, path]
    if file:
        args += ['--input', str(file), '-H', 'Content-Type: application/octet-stream']
    elif body is not None:
        args += ['--input', '-']
    result = subprocess.check_output(args, input=json.dumps(body).encode() if body is not None else None)
    return json.loads(result)


def find_release(repo, tag):
    pages = json.loads(subprocess.check_output(
        ['gh', 'api', '--paginate', '--slurp', f'repos/{repo}/releases?per_page=100']))
    matches = [r for page in pages for r in page if r['tag_name'] == tag]
    if len(matches) > 1:
        raise ValueError('Multiple releases use the requested tag')
    return matches[0] if matches else None


def main():
    repo, tag = os.environ['GITHUB_REPOSITORY'], os.environ['RELEASE_TAG']
    source = Path('release-source')
    sha = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
    if api(f'repos/{repo}/commits/{quote(tag, safe="")}')['sha'] != sha:
        raise ValueError('Release source does not match the tag')
    version = json.loads((source / 'apps/web/package.json').read_text())['version']
    if tag != f'v{version}':
        raise ValueError('Tag does not match the source version')
    release = find_release(repo, tag)
    candidate = {p.name: p.read_bytes() for p in Path('release-assets').iterdir()}
    files, backup = prepare(candidate, version, release)
    notes = (source / f'docs/releases/{tag}.md').read_text().replace('MCP Hub_', 'MCP.Hub_')
    if 'SHA256SUMS.source' in files:
        notes += '\n`SHA256SUMS.source` 为原预构建校验清单备份；下载文件请使用 `SHA256SUMS` 校验。\n'
    if not release:
        release = api(f'repos/{repo}/releases', 'POST', {
            'tag_name': tag, 'target_commitish': sha, 'name': f'MCP Hub {tag}',
            'body': notes, 'draft': True, 'prerelease': False,
        })
    endpoint = f"repos/{repo}/releases/{release['id']}"
    if backup:
        asset = next(a for a in release['assets'] if a['name'] == 'SHA256SUMS')
        renamed = api(f"repos/{repo}/releases/assets/{asset['id']}", 'PATCH', {'name': 'SHA256SUMS.source'})
        verify_asset(renamed, files)
    release = api(endpoint)
    present = {a['name'] for a in release['assets']}
    prepared = Path('prepared-assets')
    prepared.mkdir(exist_ok=True)
    for name, data in files.items():
        if name in present:
            continue
        path = prepared / name
        path.write_bytes(data)
        uploaded = api(f"https://uploads.github.com/{endpoint}/assets?name={quote(name, safe='')}", 'POST', file=path)
        verify_asset(uploaded, files)
    release = api(endpoint)
    if not release['draft'] or {a['name'] for a in release['assets']} != set(files):
        raise ValueError('Draft has an unexpected asset set or state')
    for asset in release['assets']:
        verify_asset(asset, files)
    published = api(endpoint, 'PATCH', {'body': notes, 'draft': False, 'prerelease': False})
    if published['draft'] or published['tag_name'] != tag:
        raise ValueError('Release publication did not complete')
    print(published['html_url'])


if __name__ == '__main__':
    main()
