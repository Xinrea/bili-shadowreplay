"""Regression tests for the independent Vulkan API dump verifier."""

import hashlib
from pathlib import Path
import tempfile
import unittest

import verify


class VerifyTraceTests(unittest.TestCase):
    REQUIRED_CALLS = (
        "vkCreateDevice",
        "vkCmdDrawIndexed",
        "vkQueueSubmit2",
        "vkCmdEncodeVideoKHR",
        "vkDestroyDevice",
    )

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / "api-dump.txt"

    def write_trace(self, calls=None, encoding="utf-8"):
        if calls is None:
            calls = self.REQUIRED_CALLS
        contents = "Thread 0, Frame 0:\n" + "".join(
            "{}(arguments) returns void:\n".format(name) for name in calls
        )
        self.path.write_text(contents, encoding=encoding)
        return self.path

    def test_complete_capture_reports_observed_counts_and_file_digest(self):
        calls = (
            "vkCreateDevice",
            "vkCmdDrawIndexed",
            "vkCmdDrawIndexed",
            "vkQueueSubmit2",
            "vkCmdEncodeVideoKHR",
            "vkCmdEncodeVideoKHR",
            "vkDestroyDevice",
        )
        report = verify.verify_trace(self.write_trace(calls))

        self.assertEqual(report["recorded_calls"], len(calls))
        self.assertEqual(report["draw_records"], 2)
        self.assertEqual(report["queue_submit_records"], 1)
        self.assertEqual(report["image_readback_records"], 0)
        self.assertEqual(report["command_counts"]["vkCmdEncodeVideoKHR"], 2)
        data = self.path.read_bytes()
        self.assertEqual(report["bytes"], len(data))
        self.assertEqual(report["sha256"], hashlib.sha256(data).hexdigest())
        self.assertEqual(report["path"], str(self.path.resolve()))

    def test_missing_or_empty_file_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "Missing/empty Vulkan trace"):
            verify.verify_trace(self.path)
        self.path.touch()
        with self.assertRaisesRegex(ValueError, "Missing/empty Vulkan trace"):
            verify.verify_trace(self.path)

    def test_header_without_calls_is_rejected(self):
        self.path.write_text("Thread 0, Frame 0:\nNo captured commands\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "no Vulkan call records"):
            verify.verify_trace(self.path)

    def test_each_required_capture_phase_is_required(self):
        missing_calls = {
            "vkCreateDevice": "lacks device creation",
            "vkDestroyDevice": "lacks device destruction",
            "vkCmdDrawIndexed": "lacks graphics draw calls",
            "vkQueueSubmit2": "lacks queue submissions",
            "vkCmdEncodeVideoKHR": "lacks Vulkan Video encode commands",
        }
        for missing, message in missing_calls.items():
            with self.subTest(missing=missing):
                calls = [name for name in self.REQUIRED_CALLS if name != missing]
                with self.assertRaisesRegex(ValueError, message):
                    verify.verify_trace(self.write_trace(calls))

    def test_image_to_buffer_and_host_image_readback_commands_are_rejected(self):
        readbacks = (
            "vkCmdCopyImageToBuffer",
            "vkCmdCopyImageToBuffer2",
            "vkCmdCopyImageToBuffer2KHR",
            "vkCopyImageToMemory",
            "vkCopyImageToMemoryEXT",
        )
        for command in readbacks:
            with self.subTest(command=command):
                calls = self.REQUIRED_CALLS[:-1] + (command, command, "vkDestroyDevice")
                with self.assertRaisesRegex(ValueError, "CPU image readback commands") as error:
                    verify.verify_trace(self.write_trace(calls))
                self.assertIn("'{}': 2".format(command), str(error.exception))

    def test_buffer_mapping_and_buffer_to_image_uploads_are_allowed_and_counted(self):
        allowed = (
            "vkMapMemory",
            "vkMapMemory2",
            "vkMapMemory2KHR",
            "vkCmdCopyBufferToImage",
            "vkCmdCopyBufferToImage2",
            "vkCmdCopyBufferToImage2KHR",
        )
        report = verify.verify_trace(self.write_trace(self.REQUIRED_CALLS + allowed))
        self.assertEqual(report["recorded_calls"], len(self.REQUIRED_CALLS) + len(allowed))
        self.assertEqual(report["image_readback_records"], 0)
        for command in allowed:
            with self.subTest(command=command):
                self.assertEqual(report["command_counts"][command], 1)

    def test_draw_and_submit_variants_demonstrate_gpu_work(self):
        draws = (
            "vkCmdDraw",
            "vkCmdDrawIndexed",
            "vkCmdDrawIndirect",
            "vkCmdDrawIndexedIndirect",
            "vkCmdDrawIndirectCount",
            "vkCmdDrawIndexedIndirectCountKHR",
        )
        submits = ("vkQueueSubmit", "vkQueueSubmit2", "vkQueueSubmit2KHR")
        for draw in draws:
            for submit in submits:
                with self.subTest(draw=draw, submit=submit):
                    calls = ("vkCreateDevice", draw, submit, "vkCmdEncodeVideoKHR", "vkDestroyDevice")
                    report = verify.verify_trace(self.write_trace(calls))
                    self.assertEqual(report["draw_records"], 1)
                    self.assertEqual(report["queue_submit_records"], 1)

    def test_lookup_string_does_not_count_as_image_readback(self):
        self.write_trace()
        with self.path.open("a", encoding="utf-8") as output:
            output.write('vkGetDeviceProcAddr(device, "vkCmdCopyImageToBuffer2KHR") returns PFN:\n')
            output.write('    pName: "vkCopyImageToMemoryEXT"\n')
        report = verify.verify_trace(self.path)
        self.assertEqual(report["recorded_calls"], len(self.REQUIRED_CALLS) + 1)
        self.assertEqual(report["image_readback_records"], 0)

    def test_thread_prefixed_calls_are_recognized(self):
        self.path.write_text("".join(
            "Thread 3, Frame 7: {}(arguments) returns void:\n".format(command)
            for command in self.REQUIRED_CALLS
        ), encoding="utf-8")
        report = verify.verify_trace(self.path)
        self.assertEqual(report["recorded_calls"], len(self.REQUIRED_CALLS))

    def test_utf8_and_bom_marked_utf16_captures(self):
        for encoding in ("utf-8", "utf-8-sig", "utf-16"):
            with self.subTest(encoding=encoding):
                report = verify.verify_trace(self.write_trace(encoding=encoding))
                self.assertEqual(report["recorded_calls"], len(self.REQUIRED_CALLS))
                calls = self.REQUIRED_CALLS + ("vkCopyImageToMemoryEXT",)
                with self.assertRaisesRegex(ValueError, "CPU image readback commands"):
                    verify.verify_trace(self.write_trace(calls, encoding=encoding))


if __name__ == "__main__":
    unittest.main()
