#!/usr/bin/env python3
"""Regression tests for the platform-token contract and read-only drift gate."""
import contextlib
import copy
import importlib.util
import io
import json
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("design_generator", ROOT / "scripts/gen-design-tokens.py")
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)


class DesignTokensTest(unittest.TestCase):
    def setUp(self):
        self.tokens = json.loads((ROOT / "design/brand/tokens.json").read_text())
        self.components = (ROOT / "design/brand/components.css").read_text()

    def test_every_platform_has_same_values(self):
        files = generator.outputs(ROOT, generator.validate(self.tokens), self.components)
        css_files = [value for path, value in files.items() if path.name in ("tokens.css", "design-tokens.css")]
        self.assertEqual(len(css_files), 4)
        self.assertEqual(len(set(css_files)), 1)
        css = css_files[0]
        swift = files[ROOT / "apps/wallet/Sources/DesignTokens.swift"]
        for name, color in self.tokens["color"]["light"].items():
            self.assertIn(f"--c-{name}: {color['value']}", css)
            self.assertIn(f"light: 0x{color['value'][1:]}", swift)
            self.assertIn(f"dark: 0x{self.tokens['color']['dark'][name]['value'][1:]}", swift)
        self.assertIn("static let base: Double = 0.24", swift)
        self.assertIn("--dur-base: 240ms", css)
        self.assertIn("CubicCurve(x1: 0.2, y1: 0.7, x2: 0.2, y2: 1)", swift)
        self.assertIn("iosSize: 48", swift)
        self.assertIn('family == "amount" ? base.monospacedDigit()', swift)

    def test_source_edit_reaches_every_platform(self):
        self.tokens["color"]["light"]["gold"]["value"] = "#123456"
        files = generator.outputs(ROOT, generator.validate(self.tokens), self.components)
        for path, value in files.items():
            if path.suffix == ".swift":
                self.assertIn("gold = ColorPair(light: 0x123456", value)
            elif path.name.endswith("tokens.css"):
                self.assertIn("--c-gold: #123456", value)

    def test_invalid_tokens_fail_before_generation(self):
        def change_color(t): t["color"]["dark"].pop("text")
        def change_hex(t): t["color"]["light"]["gold"]["value"] = "yellow"
        def change_motion(t): t["motion"]["duration"]["base"] = float("nan")
        def change_curve(t): t["motion"]["easing"]["standard"] = "cubic-bezier(2,0,1,1)"
        def change_shadow(t): t["shadow"]["light"]["plate"] = "0 1px 2px rgba(300,0,0,.2)"
        def change_type(t): t["font"]["scale"]["caption"]["family"] = "unknown"
        for mutation in (change_color, change_hex, change_motion, change_curve, change_shadow, change_type):
            with self.subTest(mutation=mutation.__name__):
                tokens = copy.deepcopy(self.tokens)
                mutation(tokens)
                with self.assertRaises(ValueError): generator.validate(tokens)

    def test_shadow_layers_preserve_spread_and_opacity(self):
        layers = generator.shadow_layers(self.tokens["shadow"]["light"]["window"])
        self.assertEqual(len(layers), 3)
        self.assertEqual(layers[1], (0, 30, 80, -20, [3, 11, 20], .45))

    def test_check_detects_missing_and_modified_files_without_writing(self):
        (ROOT / "tmp").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT / "tmp", prefix="design-token-test-") as temp:
            root = Path(temp)
            brand = root / "design/brand"
            brand.mkdir(parents=True)
            (brand / "tokens.json").write_text(json.dumps(self.tokens))
            (brand / "components.css").write_text(self.components)
            previous_root = generator.ROOT
            try:
                generator.ROOT = root
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    self.assertEqual(generator.main(["--check"]), 1)
                    self.assertFalse((root / "site").exists())
                    self.assertEqual(generator.main([]), 0)
                    self.assertEqual(generator.main(["--check"]), 0)
                    path = root / "apps/wallet/Sources/DesignTokens.swift"
                    path.write_text(path.read_text() + "// drift\n")
                    before = path.read_bytes()
                    self.assertEqual(generator.main(["--check"]), 1)
                    self.assertEqual(path.read_bytes(), before)
                    self.assertEqual(generator.main([]), 0)
                    self.assertEqual(generator.main(["--check"]), 0)
                    (root / "apps/explorer/design-components.css").unlink()
                    self.assertEqual(generator.main(["--check"]), 1)
            finally:
                generator.ROOT = previous_root

    def test_text_roles_meet_contrast_on_their_surfaces(self):
        def luminance(value):
            rgb = [int(value[i:i+2], 16) / 255 for i in (1, 3, 5)]
            linear = [n / 12.92 if n <= .04045 else ((n + .055) / 1.055) ** 2.4 for n in rgb]
            return sum(n * w for n, w in zip(linear, (.2126, .7152, .0722)))
        for theme in ("light", "dark"):
            colors = self.tokens["color"][theme]
            for foreground, background in (("text", "bg"), ("text-muted", "surface"), ("text-subtle", "surface"), ("plate-ink", "plate"), ("plate-soft", "plate"), ("on-accent-fill", "accent-fill")):
                pair = sorted([luminance(colors[foreground]["value"]), luminance(colors[background]["value"])])
                with self.subTest(theme=theme, foreground=foreground):
                    self.assertGreaterEqual((pair[1] + .05) / (pair[0] + .05), 4.5)


if __name__ == "__main__": unittest.main()
