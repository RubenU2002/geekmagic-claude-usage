"""Every tunable of the tray app, in one place (seconds unless it says otherwise). Modules read these as `config.NAME` at
the moment they need them, so a test can change one without touching the code that uses it."""

SETTLE_SECONDS = 5  # quiet time after a click before the slow fetch + upload starts
STALE_AFTER = 30  # a switched-to image uploaded more recently than this isn't refreshed again
RETRY_SECONDS = 10  # how often to retry while the device is unreachable
STALE_SECONDS = 180  # numbers that couldn't be refreshed for this long get the dimmed "no fresh data" image
OTHER_REFRESH = 120  # how often the provider that isn't on screen is queried (for its alerts and fresh image)
REDISCOVER_AFTER = 15  # unreachable for this long -> look for the device on the network (its router may have given it a new IP)
REDISCOVER_EVERY = 30  # ...and then at most this often (a scan takes ~4 s)
OFFLINE_NOTICE_AFTER = 45  # the screen has been gone this long (and a search for it found nothing): say so, once
DISCOVER_RETRY = 30  # while there's no address at all, how often to scan
LOCK_POLL = 3  # between checks of whether the computer got locked / unlocked
PAUSE_POLL = 5  # while paused, how often the worker looks again
LOGIN_COOLDOWN = 300  # a sign-in window isn't opened again for the same provider within this many seconds
LOGIN_POLL = 10  # while one is being signed in, how often its usage is tried again (so the screen fills in at once)
LOCAL_STATS_EVERY = 600  # the providers' local activity counts are refreshed at most this often
LOCAL_STATS_RETRY = 60  # ...or this often while one of them is still missing (it failed)
BACKGROUND_STATS = True  # count in a thread (the first count of a month of logs takes seconds); tests turn it off
AGENT_POLL = 2  # between looks at whether an agent is working (a read of a few log tails: cheap)
AGENT_IDLE_POLLS = 2  # polls in a row that must say "idle" before a run counts as finished (no flapping)
AGENT_WAIT_POLLS = 2  # polls in a row that must say "waiting for you" before it's shown and notified
AGENT_MIN_RUN = 60  # a run shorter than this ends without a notification
BACKOFF_FROM = 3  # consecutive failed reads of a provider after which it is left alone for a while
BACKOFF_BASE = 60  # ...first for this long, doubling each time it fails again
BACKOFF_MAX = 300  # ...up to this long (a click or "Actualizar ahora" tries at once)
HISTORY_MAX = 150  # readings kept per usage window, for the recent-pace estimate
HISTORY_EVERY = {"current": 300, "weekly": 1800}  # ...adding one at least this often, or when the % changed
DEFAULT_BRIGHTNESS_CHOICES = (100, 75, 50, 25, 10, 0)  # the menu's brightness levels (the device takes -10..100)
LOCK_DIM_LEVEL = 0  # backlight level while the computer is locked, if "dim when locked" is on
NIGHT_DEFAULT = {"enabled": False, "start": 22, "end": 7, "level": 10}  # 10 PM - 7 AM at 10 %
WINDOWS = (("current", "sesión"), ("weekly", "semana"))  # usage window keys -> how notifications name them
