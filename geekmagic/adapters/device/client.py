"""The GeekMagic SmallTV's own web API (stock firmware): store an image, show it, set the backlight.

The device serves one request at a time and is slow (~0.3 s per small request, ~36-46 ms per KB uploaded), which is why
the app keeps its requests few and lets a click cancel an upload in flight.
"""

from __future__ import annotations

import functools
import http.client
import re
import socket
import struct
import threading
import urllib.request

from geekmagic.core.errors import UploadCancelled, UsageError

CONNECT_TIMEOUT = 3  # seconds to reach the device / get a reply to a small request; a switched-off one fails fast
UPLOAD_TIMEOUT = 15  # seconds to let the device finish storing an uploaded GIF (~3 s in practice)
BRIGHTNESS_RANGE = (-10, 100)  # the slider range of the device's own settings page

_theme_ready: set[str] = set()  # devices this process has already put on the Photo Album theme


def _device_errors(func):
    """The device sometimes cuts a response short or answers garbage (http.client raises HTTPException,
    which isn't an OSError). Report those like any other "device didn't answer properly" failure."""
    @functools.wraps(func)
    def wrapper(*args, **kwargs):
        try:
            return func(*args, **kwargs)
        except http.client.HTTPException as e:
            raise OSError(f"incomplete or invalid response from the device ({type(e).__name__})") from e
    return wrapper


def _abort_on_cancel(cancel: threading.Event, done: threading.Event, sock) -> None:
    """Watcher: when `cancel` fires mid-upload, reset the connection so the device stops receiving at once."""
    while not done.is_set():
        if cancel.wait(0.05):
            try:
                sock.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))  # RST, drop buffered data
                sock.shutdown(socket.SHUT_RDWR)  # close() alone is deferred while the response reader holds the socket
                sock.close()
            except OSError:
                pass
            return


def _clamp_brightness(level: float) -> int:
    return max(BRIGHTNESS_RANGE[0], min(BRIGHTNESS_RANGE[1], int(level)))


class GeekMagicDevice:
    """One screen, at one address. Every method raises OSError when the device doesn't answer properly."""

    def __init__(self, ip: str) -> None:
        self.ip = ip

    # --- images ---------------------------------------------------------------------------------------------------

    @_device_errors
    def upload(self, image_bytes: bytes, filename: str, content_type: str = "image/gif",
               cancel: threading.Event | None = None) -> None:
        """POST the image to the device. Setting `cancel` aborts the transfer and raises UploadCancelled."""
        boundary = "----geekmagicclaude"
        body = (
            f"--{boundary}\r\n"
            f'Content-Disposition: form-data; name="file"; filename="{filename}"\r\n'
            f"Content-Type: {content_type}\r\n\r\n"
        ).encode() + image_bytes + f"\r\n--{boundary}--\r\n".encode()

        if cancel is not None and cancel.is_set():
            raise UploadCancelled()
        conn = http.client.HTTPConnection(self.ip, timeout=CONNECT_TIMEOUT)
        done = threading.Event()
        try:
            conn.connect()
            conn.sock.settimeout(UPLOAD_TIMEOUT)  # connecting fails fast; the device then needs seconds to store the file
            # A tiny send buffer makes us send at the device's pace instead of dumping the whole file into
            # the OS buffer at once, so a cancel can still stop the transfer while the device is busy writing.
            conn.sock.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 2048)
            if cancel is not None:
                threading.Thread(target=_abort_on_cancel, args=(cancel, done, conn.sock), daemon=True).start()
            conn.request(
                "POST", "/doUpload?dir=/image/",
                body=(body[i:i + 2048] for i in range(0, len(body), 2048)),
                headers={
                    "Content-Type": f"multipart/form-data; boundary={boundary}",
                    "Content-Length": str(len(body)),
                },
            )
            resp = conn.getresponse()
            resp.read()
            if resp.status >= 300:
                raise UsageError(f"Upload failed: HTTP {resp.status}")
        except Exception:
            if cancel is not None and cancel.is_set():
                raise UploadCancelled() from None
            raise
        finally:
            done.set()
            conn.close()

    @_device_errors
    def list_images(self) -> set[str]:
        """Names of the image files stored on the device (parsed from its file-list page)."""
        html = urllib.request.urlopen(f"http://{self.ip}/filelist?dir=/image/", timeout=CONNECT_TIMEOUT).read().decode("utf-8", "replace")
        return set(re.findall(r"[\w.\-]+\.(?:gif|jpe?g|png)", html, re.IGNORECASE))

    @_device_errors
    def delete_image(self, filename: str) -> None:
        urllib.request.urlopen(f"http://{self.ip}/delete?file=/image/{filename}", timeout=CONNECT_TIMEOUT).read()

    @_device_errors
    def show_image(self, filename: str) -> None:
        """Pin an already-uploaded image on screen. One request (~0.3 s, the device is slow): the Photo Album theme is
        only selected the first time, or again if the device ever refuses the image (the theme was changed on the
        device itself, or it restarted)."""
        try:
            if self.ip not in _theme_ready:
                self._set("theme=3")
                _theme_ready.add(self.ip)
            if self._reply(f"img=/image/{filename}") != "OK":
                self._set("theme=3")
                if self._reply(f"img=/image/{filename}") != "OK":
                    raise OSError(f"the device wouldn't show {filename}")
        except OSError:
            _theme_ready.discard(self.ip)  # it may have restarted: select the theme again next time
            raise

    # --- the backlight --------------------------------------------------------------------------------------------

    @_device_errors
    def set_brightness(self, level: float) -> None:
        """Backlight level, -10..100 (the device doesn't let you read the current one back)."""
        self._set(f"brt={_clamp_brightness(level)}")

    @_device_errors
    def set_night_mode(self, *, start_hour: int, end_hour: int, night_level: float, day_level: float, enabled: bool) -> None:
        """The device's own night schedule: between the two hours it runs at `night_level`, else at `day_level`. It keeps
        working with the computer off. (Same call as the "Night Mode" section of the device's settings page.)"""
        self._set(f"t1={start_hour % 24}&t2={end_hour % 24}&b1={_clamp_brightness(day_level)}"
                  f"&b2={_clamp_brightness(night_level)}&en={1 if enabled else 0}")

    # --- the device's /set?... settings ---------------------------------------------------------------------------

    def _reply(self, query: str) -> str:
        """GET /set?<query> and return what the device answered: "OK", or "FAIL" for what it doesn't understand."""
        return urllib.request.urlopen(f"http://{self.ip}/set?{query}", timeout=CONNECT_TIMEOUT).read().decode("utf-8", "replace").strip()

    def _set(self, query: str) -> None:
        body = self._reply(query)
        if body != "OK":
            raise OSError(f"the device refused the setting ({body or 'no answer'})")
