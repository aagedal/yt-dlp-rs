#!/usr/bin/env python3
"""A fixture-serving HTTP(S) proxy for site extractor comparisons.

Development only: serves `routes.json` of a tests/fixtures/sites/<site>
directory to any client using it as `--proxy` (plain absolute-URI requests,
and `CONNECT` tunnels terminated with the tests' self-signed certificate, so
clients need `--no-check-certificate`). No request leaves the machine; unknown
URLs are 404s. tests/helpers/site_proxy.rs implements the same routing.

routes.json: {"routes": [{"url": "https://host/path?query", "status": 200,
"headers": {"Content-Type": "..."}, "body_file": "name" | "body": "text"}]}
A route may also list "request_headers" ({"Name": "value"}) that the request
must carry; the first matching route for a URL answers (for one API endpoint
answering several clients).
"""
import base64
import json
import socket
import ssl
import tempfile
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TLS = ROOT / 'tests/fixtures/proxies/tls'


def pem(kind, der):
    text = base64.encodebytes(der).decode().replace('\n', '')
    lines = [text[i:i + 64] for i in range(0, len(text), 64)]
    return f'-----BEGIN {kind}-----\n' + '\n'.join(lines) + f'\n-----END {kind}-----\n'


def load_routes(site_dir):
    site_dir = Path(site_dir)
    data = json.loads((site_dir / 'routes.json').read_text(encoding='utf-8'))
    routes = {}
    for route in data['routes']:
        if 'body_file' in route:
            body = (site_dir / route['body_file']).read_bytes()
        else:
            body = route.get('body', '').encode()
        wanted = {name.lower(): value for name, value in route.get('request_headers', {}).items()}
        routes.setdefault(route['url'], []).append(
            (wanted, (route.get('status', 200), route.get('headers', {}), body)))
    return routes


def request_headers(head):
    headers = {}
    for line in head.split('\r\n')[1:]:
        if ':' in line:
            name, value = line.split(':', 1)
            headers[name.strip().lower()] = value.strip()
    return headers


def match_route(routes, url, headers):
    for wanted, route in routes.get(url, []):
        if all(headers.get(name) == value for name, value in wanted.items()):
            return route
    return None


class SiteProxy:
    def __init__(self, site_dir):
        self.routes = load_routes(site_dir)
        self.requests = []
        directory = tempfile.mkdtemp()
        cert, key = Path(directory, 'cert.pem'), Path(directory, 'key.pem')
        cert.write_text(pem('CERTIFICATE', (TLS / 'server.der').read_bytes()))
        key.write_text(pem('PRIVATE KEY', (TLS / 'key.der').read_bytes()))
        self.context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.context.load_cert_chain(cert, key)
        self.socket = socket.create_server(('127.0.0.1', 0))
        self.port = self.socket.getsockname()[1]
        threading.Thread(target=self._serve, daemon=True).start()

    @property
    def url(self):
        return f'http://127.0.0.1:{self.port}'

    def _serve(self):
        while True:
            client, _ = self.socket.accept()
            threading.Thread(target=self._handle, args=(client,), daemon=True).start()

    @staticmethod
    def _read_head(stream):
        data = b''
        while b'\r\n\r\n' not in data:
            chunk = stream.recv(1)
            if not chunk:
                return None
            data += chunk
        return data.decode('latin-1')

    @staticmethod
    def _read_body(stream, headers):
        remaining = int(headers.get('content-length') or 0)
        while remaining > 0:
            chunk = stream.recv(min(remaining, 65536))
            if not chunk:
                break
            remaining -= len(chunk)

    def _respond(self, stream, url, head):
        self.requests.append(url)
        headers = request_headers(head)
        self._read_body(stream, headers)
        status, headers, body = match_route(self.routes, url, headers) or (
            404, {'Content-Type': 'text/plain'}, b'not found')
        reason = {200: 'OK', 206: 'Partial Content', 302: 'Found', 403: 'Forbidden', 404: 'Not Found'}.get(status, 'OK')
        lines = [f'HTTP/1.1 {status} {reason}']
        lines += [f'{name}: {value}' for name, value in headers.items()]
        lines += [f'Content-Length: {len(body)}', 'Connection: close', '', '']
        stream.sendall('\r\n'.join(lines).encode('latin-1') + body)

    def _handle(self, client):
        try:
            head = self._read_head(client)
            if not head:
                return
            method, target, _ = head.split('\r\n', 1)[0].split(' ', 2)
            if method == 'CONNECT':
                client.sendall(b'HTTP/1.1 200 Connection established\r\n\r\n')
                tls = self.context.wrap_socket(client, server_side=True)
                inner = self._read_head(tls)
                if inner:
                    path = inner.split('\r\n', 1)[0].split(' ', 2)[1]
                    host = target.rsplit(':', 1)[0] if target.endswith(':443') else target
                    self._respond(tls, f'https://{host}{path}', inner)
                tls.close()
            else:
                self._respond(client, target, head)
        except (OSError, ValueError, ssl.SSLError):
            pass
        finally:
            client.close()
