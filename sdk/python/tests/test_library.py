# SPDX-License-Identifier: Apache-2.0
"""Loading the library: where it is looked for, and which versions an SDK accepts."""

import ctypes.util
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import ucdk._library
from ucdk import Library
from ucdk._library import SDK_ABI, accepts, split_version
from ucdk.errors import AbiMismatch, LibraryError

from .support import LIBRARY, library_path


class Loading(unittest.TestCase):
    def test_the_library_is_loaded_from_the_path_given(self):
        lib = Library(library_path())
        self.assertEqual(lib.path, LIBRARY)
        self.assertEqual(lib.abi[0], SDK_ABI[0])
        self.assertGreaterEqual(lib.abi[1], SDK_ABI[1])

    def test_a_string_path_works_too(self):
        self.assertEqual(Library(str(library_path())).abi, Library(library_path()).abi)

    def test_a_file_that_is_not_there_is_a_library_error_naming_it(self):
        with self.assertRaises(LibraryError) as caught:
            Library("/no/such/libucdk_capi.so")
        self.assertIn("/no/such/libucdk_capi.so", str(caught.exception))

    def test_without_a_path_the_package_directory_is_searched_and_nothing_else(self):
        # No ucdk library is shipped in a source checkout, and no environment variable names one.
        with self.assertRaises(LibraryError) as caught:
            Library()
        self.assertIn("pass library_path", str(caught.exception))

    def test_the_library_shipped_in_the_package_directory_is_found_without_a_path(self):
        # A platform wheel puts the library in `ucdk/_lib`; stand one up beside a stand-in for
        # the module and look from there.
        with tempfile.TemporaryDirectory() as directory:
            packaged = Path(directory) / "_lib"
            packaged.mkdir()
            shutil.copy(library_path(), packaged / library_path().name)
            with mock.patch.object(ucdk._library, "__file__", str(Path(directory) / "x.py")):
                lib = Library()
            self.assertEqual(lib.path, packaged / library_path().name)
            self.assertEqual(lib.abi, Library(library_path()).abi)

    def test_a_library_that_is_not_ucdk_is_refused_before_anything_is_called(self):
        libc = ctypes.util.find_library("c")
        if libc is None:
            self.skipTest("no C library to load")
        with self.assertRaises(LibraryError) as caught:
            Library(libc)
        self.assertIn("ucdk_abi_version", str(caught.exception))


class Versions(unittest.TestCase):
    def test_a_version_number_is_major_in_the_high_half_and_minor_in_the_low(self):
        self.assertEqual(split_version((3 << 16) | 7), (3, 7))
        self.assertEqual(split_version(0), (0, 0))

    def test_the_same_major_and_a_minor_at_least_the_sdks_is_accepted(self):
        self.assertTrue(accepts((1, 2), (1, 2)))
        self.assertTrue(accepts((1, 2), (1, 9)))

    def test_an_older_minor_or_another_major_is_not(self):
        self.assertFalse(accepts((1, 2), (1, 1)))
        self.assertFalse(accepts((1, 2), (2, 2)))
        self.assertFalse(accepts((1, 2), (0, 9)))

    def test_a_library_of_another_major_is_refused_with_both_versions_named(self):
        lib_major, lib_minor = Library(library_path()).abi
        with mock.patch.object(Library, "SDK_ABI", (lib_major + 1, 0)):
            with self.assertRaises(AbiMismatch) as caught:
                Library(library_path())
        message = str(caught.exception)
        self.assertIn("ABI %d.0" % (lib_major + 1), message)
        self.assertIn("ABI %d.%d" % (lib_major, lib_minor), message)

    def test_a_library_older_than_the_sdk_needs_is_refused(self):
        lib_major, lib_minor = Library(library_path()).abi
        with mock.patch.object(Library, "SDK_ABI", (lib_major, lib_minor + 1)):
            with self.assertRaises(AbiMismatch):
                Library(library_path())

    def test_a_library_newer_in_its_minor_than_the_sdk_is_accepted(self):
        lib_major, lib_minor = Library(library_path()).abi
        with mock.patch.object(Library, "SDK_ABI", (lib_major, max(lib_minor - 1, 0))):
            self.assertEqual(Library(library_path()).abi, (lib_major, lib_minor))

    def test_the_sdks_abi_is_the_headers(self):
        header = (LIBRARY.parents[2] / "crates/ucdk-capi/include/ucdk.h").read_text()
        major = int(header.split("#define UCDK_ABI_MAJOR ")[1].split()[0])
        minor = int(header.split("#define UCDK_ABI_MINOR ")[1].split()[0])
        self.assertEqual(SDK_ABI, (major, minor))


if __name__ == "__main__":
    unittest.main()
