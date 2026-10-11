#!/usr/bin/env python3
"""Fixture CLI test adapter: stub port allocation without opening local sockets."""
import importlib.util
from pathlib import Path
import sys

path = Path(sys.argv.pop(1))
spec = importlib.util.spec_from_file_location('fixture_cli_stub', path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ReservedPort:
    def __init__(self, index):
        self.port = 24000 + index

    def getsockname(self):
        return ('127.0.0.1', self.port)

    def close(self):
        pass


module.reserve_ports = lambda count: [ReservedPort(index) for index in range(count)]
try:
    module.main()
except (ValueError, KeyError, OSError, module.subprocess.SubprocessError):
    print('stub fixture failed', file=sys.stderr)
    sys.exit(2)
