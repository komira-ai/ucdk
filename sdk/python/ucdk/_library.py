# SPDX-License-Identifier: Apache-2.0
"""The C ABI of the ucdk library (`crates/ucdk-capi/include/ucdk.h`), bound with ctypes.

This is the whole of the binding: load the library, check its version, call the ten functions.
Nothing is read from the environment; the library is named by an argument or found in the
package directory.
"""

import ctypes
import sys
from pathlib import Path
from typing import Optional, Tuple, Union

from .errors import AbiMismatch, LibraryError, UcdkError, error_for

#: The ABI this SDK was written for, as (major, minor): the header's UCDK_ABI_MAJOR/MINOR.
SDK_ABI = (0, 2)

_NAME = "libucdk_capi.dylib" if sys.platform == "darwin" else "libucdk_capi.so"
_VERBS = ("load_catalog", "plan", "apply", "destroy", "status", "leaks")


class _Buf(ctypes.Structure):
    """`ucdk_buf`: bytes the library allocated."""

    _fields_ = [("ptr", ctypes.POINTER(ctypes.c_uint8)), ("len", ctypes.c_size_t)]


def split_version(version: int) -> Tuple[int, int]:
    """(major, minor) of `ucdk_abi_version()`, which is (major << 16) | minor."""
    return version >> 16, version & 0xFFFF


def accepts(sdk: Tuple[int, int], library: Tuple[int, int]) -> bool:
    """An SDK built for (M, m) accepts a library with the same M and a minor of at least m."""
    return library[0] == sdk[0] and library[1] >= sdk[1]


class Library:
    """A loaded ucdk library, checked for its version.

    `library_path` names the file. Without it the package directory is searched (`ucdk/_lib`,
    where a platform wheel puts the library); there is no environment variable.
    """

    #: What this SDK needs; a test may lower or raise it to see a library refused.
    SDK_ABI = SDK_ABI

    def __init__(self, library_path: Union[str, "Path", None] = None) -> None:
        path = Path(library_path) if library_path is not None else self._packaged()
        try:
            cdll = ctypes.CDLL(str(path))
        except OSError as error:
            raise LibraryError("cannot load the ucdk library %s: %s" % (path, error)) from error
        self.path = path
        self._cdll = cdll
        #: The library's ABI version, (major, minor), checked before any other call.
        self.abi = self._check_version()
        self._bind()

    @staticmethod
    def _packaged() -> Path:
        path = Path(__file__).resolve().parent / "_lib" / _NAME
        if not path.is_file():
            raise LibraryError(
                "no library_path was given and %s is not in the package; pass library_path" % path
            )
        return path

    def _check_version(self) -> Tuple[int, int]:
        try:
            function = self._cdll.ucdk_abi_version
        except AttributeError as error:
            raise LibraryError("%s does not export ucdk_abi_version" % self.path) from error
        function.argtypes = []
        function.restype = ctypes.c_uint32
        version = split_version(function())
        if not accepts(self.SDK_ABI, version):
            raise AbiMismatch(
                "this SDK needs ucdk ABI %d.%d or a later %d.x; %s is ABI %d.%d"
                % (*self.SDK_ABI, self.SDK_ABI[0], self.path, *version)
            )
        return version

    def _bind(self) -> None:
        lib = self._cdll
        self._open = lib.ucdk_session_open
        self._open.argtypes = [
            ctypes.c_char_p,
            ctypes.c_size_t,
            ctypes.POINTER(ctypes.c_void_p),
            ctypes.POINTER(_Buf),
        ]
        self._open.restype = ctypes.c_int32
        self._close = lib.ucdk_session_close
        self._close.argtypes = [ctypes.c_void_p]
        self._close.restype = None
        self._free = lib.ucdk_buf_free
        self._free.argtypes = [_Buf]
        self._free.restype = None
        self._verbs = {}
        for verb in _VERBS:
            function = getattr(lib, "ucdk_" + verb)
            function.argtypes = [
                ctypes.c_void_p,
                ctypes.c_char_p,
                ctypes.c_size_t,
                ctypes.POINTER(_Buf),
            ]
            function.restype = ctypes.c_int32
            self._verbs[verb] = function

    def _take(self, buf: _Buf) -> bytes:
        """The bytes of a buffer the library filled, which are then released."""
        if not buf.ptr:
            return b""
        data = ctypes.string_at(buf.ptr, buf.len)
        self._free(buf)
        return data

    def open_session(self, config: bytes) -> int:
        """Opens a session from a `SessionConfig`; returns its handle."""
        handle = ctypes.c_void_p()
        error = _Buf()
        status = self._open(config, len(config), ctypes.byref(handle), ctypes.byref(error))
        detail = self._take(error)
        if status != 0:
            raise error_for("ucdk_session_open", status, detail)
        # A handle is a number the library hands out; zero is never one.
        return handle.value or 0

    def close_session(self, handle: int) -> None:
        self._close(handle)

    def call(self, verb: str, handle: int, request: bytes) -> bytes:
        """Calls a verb with a request message; returns the response message's bytes."""
        function = self._verbs[verb]
        out = _Buf()
        status = function(handle, request, len(request), ctypes.byref(out))
        data = self._take(out)
        if status != 0:
            raise error_for("ucdk_" + verb, status, data)
        return data


__all__ = ["Library", "SDK_ABI", "UcdkError", "accepts", "split_version"]
