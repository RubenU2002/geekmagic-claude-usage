"""Find the GeekMagic SmallTV on the local network (stock firmware: it answers GET /space.json)."""

from __future__ import annotations

import ipaddress
import json
import socket
import urllib.request
from concurrent.futures import ThreadPoolExecutor

PROBE_TIMEOUT = 2.0  # seconds per host; hosts that don't exist simply time out. The device itself can take 2 s to answer when busy
MAX_NETWORKS = 4  # /24s scanned in one go (the main one first)


def probe(ip: str, timeout: float = PROBE_TIMEOUT) -> bool:
    """True if `ip` answers like a SmallTV: /space.json is JSON with the `total` and `free` byte counts."""
    try:
        with urllib.request.urlopen(f"http://{ip}/space.json", timeout=timeout) as resp:
            data = json.loads(resp.read(512))
        return isinstance(data, dict) and "total" in data and "free" in data
    except (OSError, ValueError):  # no route, refused, timed out, not JSON...
        return False


def local_networks() -> list[str]:
    """Private /24 prefixes this machine is on ("192.168.1."), the one used to reach the router first."""
    addresses: list[str] = []
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
            s.connect(("192.0.2.1", 9))  # sends nothing: just asks the OS which interface it would use
            addresses.append(s.getsockname()[0])
    except OSError:
        pass
    try:
        addresses += [info[4][0] for info in socket.getaddrinfo(socket.gethostname(), None, socket.AF_INET)]
    except OSError:
        pass
    prefixes: list[str] = []
    for address in addresses:
        ip = ipaddress.ip_address(address)
        if ip.is_private and not ip.is_loopback and not ip.is_link_local:
            prefix = address.rsplit(".", 1)[0] + "."
            if prefix not in prefixes:
                prefixes.append(prefix)
    return prefixes[:MAX_NETWORKS]


def scan(hosts: list[str] | None = None, timeout: float = PROBE_TIMEOUT, workers: int = 128) -> list[str]:
    """Every host in `hosts` (default: all of this machine's /24 networks, plus the device's own hotspot
    address) that answers like a SmallTV. The main network is scanned first and, if it has one, we stop there."""
    if hosts is not None:
        batches = [hosts]
    else:
        batches = [[f"{prefix}{n}" for n in range(1, 255)] for prefix in local_networks()]
        batches.append(["192.168.4.1"])  # what the device itself answers on when you're on its own WiFi
    for batch in batches:
        with ThreadPoolExecutor(max_workers=workers) as pool:
            found = [ip for ip, ok in zip(batch, pool.map(lambda h: probe(h, timeout), batch)) if ok]
        if found:
            return found
    return []


def find_device(prefer: str | None = None) -> str | None:
    """The device's IP: `prefer` if it still answers, else the first one a scan finds."""
    if prefer and probe(prefer, 8.0):  # (a busy device is slow, not gone)
        return prefer
    found = scan()
    if prefer in found:
        return prefer
    return found[0] if found else None


class Locator:
    """The DeviceLocator port, over the functions above."""

    def probe(self, host: str, timeout: float = PROBE_TIMEOUT) -> bool:
        return probe(host, timeout)

    def find_device(self, prefer: str | None = None) -> str | None:
        return find_device(prefer)

    def networks(self) -> list[str]:
        return local_networks()
