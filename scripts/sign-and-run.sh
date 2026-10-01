#!/bin/sh
# Cargo runner: sign the app binary with a stable identity, then exec it.
#
# An unsigned cargo build gets a new ad-hoc code hash on every rebuild, so the
# macOS keychain "Always Allow" grant never sticks and it asks for your password
# on every launch. Signing with a fixed identity keeps the grant valid.
#
# Identity: $MAIL_CLASSIFIER_SIGN_IDENTITY, else the first valid code-signing
# identity in the keychain. With none available the binary runs unsigned.
# Only the app binary is signed; test binaries run as-is.
set -eu

bin="$1"

if [ "$(uname)" = Darwin ] && [ "$(basename "$bin")" = mail-classifier ]; then
    identity="${MAIL_CLASSIFIER_SIGN_IDENTITY:-}"
    if [ -z "$identity" ]; then
        identity=$(security find-identity -v -p codesigning 2>/dev/null \
            | sed -n 's/^ *1) \([0-9A-F]\{40\}\) .*/\1/p')
    fi
    if [ -n "$identity" ]; then
        codesign -f -s "$identity" "$bin" >/dev/null 2>&1 \
            || echo "warning: codesign failed; keychain may prompt again" >&2
    fi
fi

exec "$@"
