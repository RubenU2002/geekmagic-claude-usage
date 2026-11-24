"""The architecture is a rule about imports (hexagonal: ports and adapters), and this test is the rule.

    core         imports nothing of the rest, and no third-party library: what the app is about, and its ports
    application  imports core only, and no third-party library: the use cases, written against the ports
    adapters     implement the ports; each imports core and its own package, never another adapter
                 (the tray, which drives the application, may also import application)
    entry points (bootstrap, cli, tray_main) are the only places that know everything and wire it up

The diagrams in ARCHITECTURE.md are this table.
"""
import ast
import sys
import unittest
from pathlib import Path

PACKAGE = Path(__file__).resolve().parents[1] / "geekmagic"

ENTRY_POINTS = {"bootstrap", "cli", "tray_main", "paths"}  # (paths: where the app keeps its own files; read by the wiring)

# the geekmagic layers a layer may import from
ALLOWED = {
    "core": {"core"},
    "application": {"core", "application"},
    "adapters": {"core"},  # plus its own package, see layer_of
}
THIRD_PARTY = {
    "core": set(), "application": set(),
    "adapters/render": {"PIL"}, "adapters/tray": {"PIL", "pystray"},
    "adapters/system": {"Quartz"},  # macOS's own API, for the lock sensor
}
STDLIB = set(sys.stdlib_module_names)


def layer_of(relative: Path) -> str | None:
    """The layer a file belongs to: core, application, adapters/<name>, or None for an entry point."""
    parts = relative.with_suffix("").parts
    if parts[0] in ("core", "application"):
        return parts[0]
    if parts[0] == "adapters":
        return f"adapters/{parts[1]}" if len(parts) > 2 else None
    return None


def imported_modules(path: Path) -> set[str]:
    found = set()
    for node in ast.walk(ast.parse(path.read_text(encoding="utf-8"))):
        if isinstance(node, ast.ImportFrom) and node.module:
            found.add(node.module)
            found.update(f"{node.module}.{alias.name}" for alias in node.names)
        elif isinstance(node, ast.Import):
            found.update(alias.name for alias in node.names)
    return found


def violations() -> list[str]:
    problems = []
    for path in sorted(PACKAGE.rglob("*.py")):
        relative = path.relative_to(PACKAGE)
        layer = layer_of(relative)
        if layer is None:
            continue
        family = layer.split("/")[0]
        for module in imported_modules(path):
            top = module.split(".")[0]
            if top == "geekmagic":
                parts = module.split(".")
                target = parts[1] if len(parts) > 1 else ""
                if target in ("core", "application"):
                    ok = target in ALLOWED[family] or (layer == "adapters/tray" and target == "application")
                elif target == "adapters":
                    own = f"adapters/{parts[2]}" if len(parts) > 2 else ""
                    ok = family == "adapters" and own == layer
                else:
                    ok = False  # bootstrap, cli... : nothing below the entry points may reach up to them
                if not ok:
                    problems.append(f"{relative}: {layer} imports {module}")
            elif top not in STDLIB and top not in ("__future__",) and top not in THIRD_PARTY.get(layer, THIRD_PARTY.get(family, set())):
                problems.append(f"{relative}: {layer} imports the third-party {top}")
    return problems


class ArchitectureTests(unittest.TestCase):
    def test_every_layer_only_imports_what_the_rule_allows(self):
        self.assertEqual(violations(), [])

    def test_every_module_belongs_to_a_layer_or_is_an_entry_point(self):
        for path in PACKAGE.rglob("*.py"):
            relative = path.relative_to(PACKAGE)
            if relative.name == "__init__.py" and len(relative.parts) == 1:
                continue
            top = relative.parts[0] if len(relative.parts) > 1 else relative.stem
            self.assertIn(top, {"core", "application", "adapters"} | ENTRY_POINTS, f"{relative} is outside the architecture")
            if top == "adapters" and len(relative.parts) > 2:
                self.assertIn(relative.parts[1], {"providers", "render", "device", "system", "store", "tray"})

    def test_the_application_is_driven_through_ports_it_does_not_construct(self):
        """It names no adapter class: those only appear in bootstrap."""
        for path in (PACKAGE / "application").rglob("*.py"):
            self.assertFalse([m for m in imported_modules(path) if m.startswith("geekmagic.adapters")], path.name)


if __name__ == "__main__":
    unittest.main()
