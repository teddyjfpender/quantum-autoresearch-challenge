#!/usr/bin/env python3
"""Download pinned integrals and only the two needed members of spinfree.zip."""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import struct
import urllib.request
import zipfile
import zlib

from inputs import HERE, checked


def download(url, start=None, size=None):
    headers = {}
    if start is not None:
        headers['Range'] = f'bytes={start}-{start+size-1}'
        # Some HTTP caches fail to vary on Range; make ranges distinct URLs.
        url += '&range_start=' + str(start)
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=120) as r:
        if start is not None:
            prefix = f'bytes {start}-{start+size-1}/'
            if r.status != 206 or not r.headers.get('Content-Range', '').startswith(prefix):
                raise ValueError('server did not honor exact byte range')
        data = r.read() if size is None else r.read(size+1)
    if size is not None and len(data) != size:
        raise ValueError('truncated or oversized range')
    return data


def save_checked(path, data, digest):
    if hashlib.sha256(data).hexdigest() != digest:
        raise ValueError(f'download hash mismatch: {path.name}')
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_bytes(data)
    temporary.replace(path)


def fetch(directory):
    directory.mkdir(parents=True, exist_ok=True)
    sources = json.loads((HERE/'sources.json').read_text())
    for name, s in sources['instances'].items():
        path = directory/(name+'.pickle')
        if path.exists():
            checked(path, s['factors_sha256'])
            continue
        url, offset = sources['factors_archive']['url'], s['factor_offset']
        header = download(url, offset, 30)
        fields = struct.unpack('<4s5H3I2H', header)
        if fields[0] != b'PK\x03\x04' or fields[3] != 8 or fields[2] & 1:
            raise ValueError('unsupported ZIP member')
        nlen, xlen = fields[-2:]
        body = download(url, offset+30, nlen+xlen+s['factor_compressed_size'])
        if body[:nlen].decode() != s['factor_member']:
            raise ValueError('ZIP member name mismatch')
        data = zlib.decompress(body[nlen+xlen:], -15)
        if len(data) != s['factor_size'] or zlib.crc32(data) != s['factor_crc32']:
            raise ValueError('ZIP member checksum/size mismatch')
        save_checked(path, data, s['factors_sha256'])
        print(f'fetched {name} factors', flush=True)
    missing = [s for s in sources['instances'].values() if not (directory/s['integrals']).exists()]
    if missing:
        archive = directory/'integrals.zip'
        if not archive.exists():
            archive.write_bytes(download(sources['integrals_archive']['url']))
        if hashlib.md5(archive.read_bytes()).hexdigest() != sources['integrals_archive']['md5']:
            raise ValueError('integrals archive checksum mismatch')
        with zipfile.ZipFile(archive) as z:
            for s in missing:
                # Extract only explicit members, never arbitrary archive paths.
                save_checked(directory/s['integrals'], z.read('integrals/'+s['integrals']), s['integrals_sha256'])
    for s in sources['instances'].values():
        checked(directory/s['integrals'], s['integrals_sha256'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data', type=pathlib.Path, required=True)
    fetch(parser.parse_args().data)
