"""What can go wrong reading a provider's usage."""


class UsageError(RuntimeError):
    pass


class SignInNeeded(UsageError):
    """The provider can't be read because you're signed out (or its session expired): signing in again fixes it."""


class UploadCancelled(Exception):
    """An upload to the screen was aborted because something more urgent (a click) needed it."""
