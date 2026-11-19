#!/usr/bin/env bash
# Stands in for the server in entrypoint.spec.ts. It keeps one SQLite
# connection open in WAL mode with automatic checkpoints off, so new rows stay
# in the -wal file the way they do under the real server, and writes a row
# every 50 ms until SIGTERM.
set -u
db="$WA_DATA_DIR/messages.db"
rows="$(sqlite3 "$db" "CREATE TABLE IF NOT EXISTS rows(n INTEGER); SELECT count(*) FROM rows;")"
echo "started rows=$rows host=$(cat "$WA_DATA_DIR/chatgpt-host.json" 2>/dev/null)" >>"$FAKE_SERVER_LOG"
printf 'host-id' >"$WA_DATA_DIR/chatgpt-host.json"

fifo="$(mktemp -u)"
mkfifo "$fifo"
sqlite3 "$db" <"$fifo" >/dev/null &
writer=$!
exec 3>"$fifo"
echo "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;" >&3

stop=0
trap 'stop=1' TERM
while [ "$stop" = 0 ]; do
	echo "INSERT INTO rows VALUES (randomblob(300));" >&3
	sleep 0.05
done
exec 3>&-
wait "$writer"
rm -f "$fifo"
echo "stopped" >>"$FAKE_SERVER_LOG"
