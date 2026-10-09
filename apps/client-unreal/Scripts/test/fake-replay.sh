#!/usr/bin/env bash
# Stand-in for nightfall-replay: fails on a recording containing DIVERGE.
! grep -q DIVERGE "$1"
