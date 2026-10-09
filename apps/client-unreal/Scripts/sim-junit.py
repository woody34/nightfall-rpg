#!/usr/bin/env python3
"""Validate a report, record pipeline failures, or merge finalized per-client reports."""
from pathlib import Path
import sys
import xml.etree.ElementTree as ET


def read(path):
    root = ET.parse(path).getroot()
    suites = [root] if root.tag == "testsuite" else list(root.iter("testsuite"))
    if not suites:
        raise ValueError("no testsuite element")
    for suite in suites:
        for key in ("tests", "failures", "errors"):
            int(suite.get(key, "0"))
    return root, suites


def check(path):
    try:
        root, suites = read(path)
        return any(int(s.get("failures", "0")) or int(s.get("errors", "0")) for s in suites) or bool(
            root.findall(".//failure") or root.findall(".//error"))
    except (OSError, ET.ParseError, ValueError):
        return True


def failure(path, name, message):
    try:
        root, suites = read(path)
    except (OSError, ET.ParseError, ValueError):
        root = ET.Element("testsuite", name=name, tests="0", failures="0", errors="0")
        suites = [root]
        message = "missing or unreadable JUnit report; " + message
    suite = suites[0]
    case = ET.SubElement(suite, "testcase", classname=name, name="pipeline")
    ET.SubElement(case, "failure", message=message).text = message
    suite.set("tests", str(int(suite.get("tests", "0")) + 1))
    suite.set("failures", str(int(suite.get("failures", "0")) + 1))
    if root.tag == "testsuites":
        for key in ("tests", "failures", "errors"):
            root.set(key, str(sum(int(s.get(key, "0")) for s in suites)))
    ET.ElementTree(root).write(path, encoding="utf-8", xml_declaration=True)


if __name__ == "__main__":
    operation, *args = sys.argv[1:]
    if operation == "check":
        sys.exit(check(Path(args[0])))
    elif operation == "failure":
        failure(Path(args[0]), args[1], args[2])
    elif operation == "merge":
        destination, group, *reports = args
        root = ET.Element("testsuites", name=group)
        for path in reports:
            for suite in read(Path(path))[1]:
                suite.set("group", group)
                root.append(suite)
        for key in ("tests", "failures", "errors"):
            root.set(key, str(sum(int(s.get(key, "0")) for s in root)))
        ET.ElementTree(root).write(destination, encoding="utf-8", xml_declaration=True)
    else:
        raise SystemExit(f"unknown operation: {operation}")
