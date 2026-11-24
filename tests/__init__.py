"""The tests. Run them from the repo root with `python -m unittest`."""
import os

# No test may pop up a real desktop notification (notifier.send honours this).
os.environ.setdefault("GEEKMAGIC_NO_NOTIFY", "1")
# ...and none may open a real sign-in window (login.launch honours this; the tests of login itself clear it).
os.environ.setdefault("GEEKMAGIC_NO_LOGIN", "1")
