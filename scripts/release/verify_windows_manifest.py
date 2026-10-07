#!/usr/bin/env python3
"""Check embedded launch manifests, including Velopack's portable stub."""
import argparse
from pathlib import Path
import xml.etree.ElementTree as ET
import zipfile

import pefile

ASM = "{urn:schemas-microsoft-com:asm.v3}"
COMPAT = "{urn:schemas-microsoft-com:compatibility.v1}"
WINDOWS_10_11 = "{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"


def verify(data: bytes, label: str) -> None:
    with pefile.PE(data=data, fast_load=True) as pe:
        pe.parse_data_directories(
            directories=[pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_RESOURCE"]]
        )
        manifests = []
        resources = getattr(pe, "DIRECTORY_ENTRY_RESOURCE", None)
        for resource in resources.entries if resources else ():
            if resource.id != 24:  # RT_MANIFEST
                continue
            for name in resource.directory.entries:
                if name.id != 1:  # CREATEPROCESS_MANIFEST_RESOURCE_ID
                    continue
                for language in name.directory.entries:
                    entry = language.data.struct
                    xml = pe.get_data(entry.OffsetToData, entry.Size)
                    manifests.append(ET.fromstring(xml))
        if not manifests:
            raise ValueError(f"{label}: no embedded process manifest")
        for manifest in manifests:
            levels = manifest.findall(
                f"./{ASM}trustInfo/{ASM}security/"
                f"{ASM}requestedPrivileges/{ASM}requestedExecutionLevel"
            )
            expected = {"level": "asInvoker", "uiAccess": "false"}
            if len(levels) != 1 or levels[0].attrib != expected:
                raise ValueError(f"{label}: expected asInvoker with uiAccess=false")
            supported = manifest.findall(
                f"./{COMPAT}compatibility/{COMPAT}application/{COMPAT}supportedOS"
            )
            if not any(node.get("Id") == WINDOWS_10_11 for node in supported):
                raise ValueError(f"{label}: missing Windows 10/11 compatibility declaration")
    print(f"Verified Windows manifest: {label}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path)
    parser.add_argument(
        "--main-exe",
        help="Verify both root stub and current executable in a portable ZIP",
    )
    args = parser.parse_args()
    if args.main_exe:
        with zipfile.ZipFile(args.path) as archive:
            for member in (args.main_exe, f"current/{args.main_exe}"):
                verify(archive.read(member), f"{args.path}:{member}")
    else:
        verify(args.path.read_bytes(), str(args.path))


if __name__ == "__main__":
    main()
