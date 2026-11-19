#!/usr/bin/env bash
# Entrypoint of the server image. Without WA_BACKUP_URL (local Docker runs) it
# just runs the server.
#
# On Cloudflare Containers the Worker sets WA_BACKUP_URL. The container disk
# does not survive a restart, so this keeps WA_DATA_DIR in R2 through the
# backup gateway at that URL (apps/cloudflare/src/backup-store.ts):
#
# 1. Restore the latest backup into WA_DATA_DIR. With no backup yet, start
#    empty. If the restore fails, exit instead, so a new empty data directory
#    can never replace the real backup.
# 2. Run the server, and back up every WA_BACKUP_INTERVAL_SECONDS when a file
#    in WA_DATA_DIR was written, added or removed.
# 3. When the server exits (on SIGTERM or otherwise), upload a final backup if
#    anything changed, and exit. Unchanged data is never uploaded again, so a
#    crash loop cannot push the last good backups out of R2.
set -uo pipefail

SERVER_BIN="${WA_SERVER_BIN:-/app/whatsapp-translator}"
if [ -z "${WA_BACKUP_URL:-}" ]; then
	exec "$SERVER_BIN" "$@"
fi
: "${WA_DATA_DIR:?WA_DATA_DIR must be set}"
INTERVAL="${WA_BACKUP_INTERVAL_SECONDS:-300}"
if ! [[ "$INTERVAL" =~ ^[1-9][0-9]*$ ]]; then
	INTERVAL=300
fi
# R2 multipart parts must all be the same size except the last, and each
# request from the container must stay under 100 MiB.
PART_BYTES="${WA_BACKUP_PART_BYTES:-33554432}"

WORK="$(mktemp -d)"
MARKER="$WORK/last-backup"
LISTING="$WORK/last-listing"
STOP_FILE="$WORK/stop"
SERVER_PID=""
LOOP_PID=""

log() { echo "cloudflare-entrypoint: $*" >&2; }

http() {
	# --fail-with-body keeps the gateway's error text for the log.
	curl --silent --show-error --fail-with-body --retry 4 --retry-all-errors \
		--retry-delay 2 --max-time 600 "$@"
}

restore() {
	local archive="$WORK/restore.tar.gz" status
	mkdir -p "$WA_DATA_DIR"
	status="$(curl --silent --show-error --retry 4 --retry-delay 2 --max-time 600 \
		--output "$archive" --write-out '%{http_code}' "$WA_BACKUP_URL/backup")" || {
		log "restore failed: the backup gateway did not answer"
		return 1
	}
	case "$status" in
	200)
		if ! gzip --test "$archive" || ! tar -xzf "$archive" -C "$WA_DATA_DIR"; then
			log "restore failed: the backup archive is not readable"
			return 1
		fi
		log "restored $WA_DATA_DIR from the latest backup ($(stat -c %s "$archive") bytes)"
		;;
	404)
		log "no backup yet; starting with an empty data directory"
		;;
	*)
		log "restore failed: the backup gateway answered HTTP $status"
		return 1
		;;
	esac
	rm -f "$archive"
	listing >"$LISTING"
	touch "$MARKER"
}

# The data files a backup holds, one per line. Voice-note scratch directories
# (voice-tmp-*) and SQLite's WAL and journal files are left out.
listing() {
	find "$WA_DATA_DIR" -type f ! -path "$WA_DATA_DIR/voice-tmp-*" \
		! -name '*-wal' ! -name '*-shm' ! -name '*-journal' -printf '%P\n' | LC_ALL=C sort
}

# Whether any data file was written, added or removed since the last backup.
changed() {
	[ -e "$MARKER" ] || return 0
	[ -n "$(find "$WA_DATA_DIR" -type f ! -path "$WA_DATA_DIR/voice-tmp-*" -newer "$MARKER" -print -quit)" ] && return 0
	! listing | cmp -s - "$LISTING"
}

# Copies the files listing() names into $1, using SQLite's online backup for
# databases so the copy is consistent while the server writes; each copy is a
# complete database, so no WAL file is needed. A file the server deletes while
# this runs is skipped.
snapshot() {
	local dest="$1" rel
	mkdir -p "$dest"
	while IFS= read -r rel; do
		mkdir -p "$dest/$(dirname "$rel")"
		if [[ "$rel" == *.db ]]; then
			sqlite3 -bail -cmd ".timeout 30000" "$WA_DATA_DIR/$rel" ".backup '$dest/$rel'" ||
				[ ! -e "$WA_DATA_DIR/$rel" ] || return 1
		else
			cp -p "$WA_DATA_DIR/$rel" "$dest/$rel" 2>/dev/null ||
				[ ! -e "$WA_DATA_DIR/$rel" ] || return 1
		fi
	done <"$WORK/listing.new"
}

upload() {
	local archive="$1" token part n=0 parts="$WORK/parts.jsonl"
	token="$(http -X POST "$WA_BACKUP_URL/backup/uploads")" || return 1
	rm -f "$WORK"/part-* "$parts"
	split --bytes="$PART_BYTES" --numeric-suffixes=1 --suffix-length=5 \
		"$archive" "$WORK/part-" || return 1
	for part in "$WORK"/part-*; do
		n=$((n + 1))
		if ! http -X PUT --data-binary "@$part" \
			"$WA_BACKUP_URL/backup/uploads/$token/parts/$n" >>"$parts"; then
			http -X DELETE "$WA_BACKUP_URL/backup/uploads/$token" >/dev/null || true
			return 1
		fi
		echo >>"$parts"
	done
	rm -f "$WORK"/part-*
	{
		printf '['
		paste -sd, "$parts"
		printf ']'
	} | http -X POST -H 'content-type: application/json' --data-binary @- \
		"$WA_BACKUP_URL/backup/uploads/$token/complete" >/dev/null
}

# Backs up WA_DATA_DIR when it changed since the last backup.
backup() {
	local started="$WORK/started" snap="$WORK/snapshot" archive="$WORK/backup.tar.gz"
	changed || return 0
	touch "$started"
	listing >"$WORK/listing.new"
	rm -rf "$snap" "$archive"
	if ! snapshot "$snap"; then
		log "backup failed: could not copy the data directory"
		rm -rf "$snap"
		return 1
	fi
	if ! tar -czf "$archive" -C "$snap" .; then
		log "backup failed: could not write the archive"
		rm -rf "$snap" "$archive"
		return 1
	fi
	rm -rf "$snap"
	if ! upload "$archive"; then
		log "backup failed: upload to R2 did not complete"
		rm -f "$archive"
		return 1
	fi
	log "backed up $WA_DATA_DIR ($(stat -c %s "$archive") bytes)"
	rm -f "$archive"
	# Changes made while this backup ran are newer than the marker, so the
	# next round picks them up.
	mv "$WORK/listing.new" "$LISTING"
	mv "$started" "$MARKER"
}

backup_loop() {
	local waited
	while [ ! -e "$STOP_FILE" ]; do
		waited=0
		while [ "$waited" -lt "$INTERVAL" ] && [ ! -e "$STOP_FILE" ]; do
			sleep 1
			waited=$((waited + 1))
		done
		[ -e "$STOP_FILE" ] && break
		backup || true
	done
}

# shellcheck disable=SC2317 # Called by the trap below.
on_term() {
	log "stopping the server"
	[ -n "$SERVER_PID" ] && kill -TERM "$SERVER_PID" 2>/dev/null
}

if ! restore; then
	exit 1
fi

trap on_term TERM INT

"$SERVER_BIN" "$@" &
SERVER_PID=$!
backup_loop &
LOOP_PID=$!

# wait returns early when a trapped signal arrives, so keep waiting until the
# server has actually exited.
wait "$SERVER_PID"
status=$?
while kill -0 "$SERVER_PID" 2>/dev/null; do
	wait "$SERVER_PID"
	status=$?
done
log "server exited with status $status"

touch "$STOP_FILE"
wait "$LOOP_PID" 2>/dev/null
if ! backup; then
	log "final backup failed; changes since the last backup are lost"
fi
rm -rf "$WORK"
exit "$status"
