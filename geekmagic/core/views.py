"""The views of two providers at once: their keys, and what the application needs to know about them.

How each is drawn is the Renderer's business (an adapter); every Renderer must be able to draw all of these.
"""

SPLIT = "split"  # both providers on one screen; every view has its own image slots on the device
STATS = "stats"  # activity numbers for both providers
BREAKDOWN = "breakdown"  # which projects and models the requests went to
HOURS = "hours"  # at what hours of the day you work

PANEL_KEYS = (SPLIT, STATS, BREAKDOWN, HOURS)
VIEW_NAMES = {SPLIT: "vista dividida", STATS: "estadísticas", BREAKDOWN: "proyectos y modelos", HOURS: "horas pico"}
STATS_VIEWS = frozenset({STATS, BREAKDOWN, HOURS})  # drawn from the providers' local activity logs (counted in the background)
