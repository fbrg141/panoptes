#!/bin/sh
# One real round trip with the installed Pi; fails if the event or history
# contract broke. Run it after changing the adapter, since the check only
# talks to the fake Pi.
set -eu
cd "$(dirname "$0")/.."
exec cargo run --quiet -p panoptes-pi --example smoke
