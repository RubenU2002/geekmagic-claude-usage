"""Device discovery against local fake servers (no real device needed)."""
import json
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

from geekmagic.adapters.device import discovery


def serve(body: bytes, content_type: str = "application/json"):
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"127.0.0.1:{server.server_port}"


class DiscoverTests(unittest.TestCase):
    def setUp(self):
        self.servers = []

    def tearDown(self):
        for server in self.servers:
            server.shutdown()
            server.server_close()

    def fake(self, body, content_type="application/json"):
        server, host = serve(body, content_type)
        self.servers.append(server)
        return host

    def test_probe_accepts_a_smalltv_style_space_json(self):
        host = self.fake(json.dumps({"total": 3121152, "free": 1015308}).encode())
        self.assertTrue(discovery.probe(host, 2))

    def test_probe_rejects_other_servers(self):
        self.assertFalse(discovery.probe(self.fake(b"<html>router login</html>", "text/html"), 2))
        self.assertFalse(discovery.probe(self.fake(json.dumps({"hello": "world"}).encode()), 2))
        self.assertFalse(discovery.probe("127.0.0.1:1", 0.5))  # nothing listening

    def test_scan_returns_only_the_devices(self):
        device = self.fake(json.dumps({"total": 1, "free": 1}).encode())
        router = self.fake(b"<html></html>", "text/html")
        self.assertEqual(discovery.scan([router, "127.0.0.1:1", device], timeout=2), [device])
        self.assertEqual(discovery.scan([router, "127.0.0.1:1"], timeout=0.5), [])

    def test_local_networks_are_private_prefixes(self):
        for prefix in discovery.local_networks():
            self.assertRegex(prefix, r"^\d+\.\d+\.\d+\.$")
            self.assertTrue(prefix.startswith(("10.", "192.168.", "172.")))
        self.assertLessEqual(len(discovery.local_networks()), discovery.MAX_NETWORKS)


if __name__ == "__main__":
    unittest.main()
