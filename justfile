#!/usr/bin/env just -f

import? "justfile.mise"

# This help
[group('misc')]
@help:
    [ -f justfile.mise ] || just justise
    just -l -u

# Convert mise tasks to just recipes
[group('misc')]
@justise:
    mise run justise
