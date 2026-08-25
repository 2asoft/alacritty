from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from report import observe, response_sequence


class ResponseSequenceTest(unittest.TestCase):
    def test_classifies_ordered_protocol_responses(self) -> None:
        response = (
            b"\x1b_Gi=31;OK\x1b\\"
            b"\x1b[4;480;640t"
            b"\x1b[6;20;9t"
            b"\x1b[8;24;80t"
            b"\x1b[?6c"
        )

        sequence = response_sequence(response)

        self.assertEqual(
            sequence,
            (
                "kgp-ok",
                "text-area-pixels",
                "cell-size-pixels",
                "screen-size-cells",
                "primary-device-attributes",
            ),
        )

    def test_unavailable_terminal_does_not_invent_render_observations(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            result = observe(
                Path(temporary_directory), "alacritty", "unavailable", available=False
            )

        self.assertFalse(result.available)
        self.assertEqual(result.response_sequence, ())
        self.assertIsNone(result.kgp_response)
        self.assertIsNone(result.text_area_pixel_response)
        self.assertIsNone(result.cell_size_response)
        self.assertIsNone(result.screen_size_response)
        self.assertIsNone(result.primary_device_attributes_response)
        self.assertIsNone(result.primary_red_pixels)
        self.assertIsNone(result.virtual_green_pixels)
        self.assertIsNone(result.leaked_placeholder_magenta_pixels)

    def test_records_unexpected_and_incomplete_responses(self) -> None:
        response = b"x\x1b[1;2R\x1b_Gi=31;EINVAL\x1b\\\x1b["

        sequence = response_sequence(response)

        self.assertEqual(
            sequence,
            ("unparsed-byte", "csi-other", "kgp-other", "incomplete-csi"),
        )


if __name__ == "__main__":
    unittest.main()
