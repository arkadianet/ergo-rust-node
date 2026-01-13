#!/bin/bash

# Ergo Rust Node Sync Monitor
# Logs sync progress every 15 minutes to sync-progress.log

LOG_FILE="sync-progress.log"
API_URL="http://127.0.0.1:9053/info"
INTERVAL=900  # 15 minutes in seconds

echo "Ergo Rust Node Sync Monitor"
echo "Logging to: $LOG_FILE"
echo "Polling every 15 minutes"
echo "Press Ctrl+C to stop"
echo ""

# Write header to log file
echo "timestamp,headers_height,full_height,peer_count,is_synced" > "$LOG_FILE"

while true; do
    TIMESTAMP=$(date '+%Y-%m-%d %H:%M:%S')

    # Call the API
    RESPONSE=$(curl -s "$API_URL" 2>/dev/null)

    if [ $? -eq 0 ] && [ -n "$RESPONSE" ]; then
        # Parse JSON response
        HEADERS=$(echo "$RESPONSE" | jq -r '.headersHeight // "N/A"')
        FULL=$(echo "$RESPONSE" | jq -r '.fullHeight // "N/A"')
        PEERS=$(echo "$RESPONSE" | jq -r '.peerCount // "N/A"')
        SYNCED=$(echo "$RESPONSE" | jq -r '.isSynced // "N/A"')

        # Log to file
        echo "$TIMESTAMP,$HEADERS,$FULL,$PEERS,$SYNCED" >> "$LOG_FILE"

        # Print to console
        echo "[$TIMESTAMP] Headers: $HEADERS | Full blocks: $FULL | Peers: $PEERS | Synced: $SYNCED"
    else
        echo "$TIMESTAMP,ERROR,ERROR,ERROR,ERROR" >> "$LOG_FILE"
        echo "[$TIMESTAMP] ERROR: Could not reach API at $API_URL"
    fi

    sleep $INTERVAL
done
