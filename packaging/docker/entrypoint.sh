#!/bin/sh
# The daemon needs a token to accept RPC calls; the CLI health probe and every
# client use the same one, so refuse to start without it instead of listening
# with an empty secret.
set -eu

if [ -z "${FERRENT_TOKEN:-}" ]; then
    echo "FERRENT_TOKEN is not set" >&2
    exit 1
fi

exec /usr/bin/ferrentd "$@"
