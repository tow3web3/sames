#!/bin/bash
# Grind for vanity keypairs ending in "same" (case-sensitive)
# Saves keypairs to /root/obly/sames/vanity-keys/
export PATH="$HOME/.local/share/solana/install/active_release/bin:$HOME/.cargo/bin:$PATH"
KEYDIR="/root/obly/sames/vanity-keys"
mkdir -p "$KEYDIR"
cd "$KEYDIR"

TARGET=${1:-50}
COUNT=0

while [ $COUNT -lt $TARGET ]; do
    solana-keygen grind --ends-with same:1 2>/dev/null
    if [ $? -eq 0 ]; then
        COUNT=$((COUNT + 1))
        echo "[$(date)] Found #$COUNT"
    fi
done
echo "Done! Ground $COUNT keypairs."
