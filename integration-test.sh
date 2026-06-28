#!/usr/bin/env bash
set -euxo pipefail

# See [ref:integration_test_step] for how to run this integration test.

# Wait for the Docker daemon to start up.
echo 'Waiting for Docker to start…'
while ! docker container ls > /dev/null 2>&1; do
  sleep 1
done

# Start Docuum in the background, redirecting its output to a log file so we
# can synchronize with it by watching for specific log messages.
echo 'Starting Docuum…'
DOCUUM_LOG="$(mktemp)"
LOG_LEVEL=debug /docuum-x86_64-unknown-linux-musl --threshold '20 MB' --keep 'alpine:keep' \
  >"$DOCUUM_LOG" 2>&1 &
DOCUUM_PID="$!"

# Wait for Docuum to finish its initial vacuum and start listening for events.
wait_for_startup() {
  echo 'Waiting for Docuum to start listening…'
  until grep -q 'Listening for Docker events' "$DOCUUM_LOG" 2>/dev/null; do
    sleep 0.5
  done
}

# Wait for Docuum to finish processing all pending Docker events. This is more
# reliable than polling the process state (which only tells us the process is
# blocked on *some* syscall, not necessarily the event-read). We wait for new
# "Going back to sleep" log messages (emitted after each event is fully
# processed, including any vacuum), then wait for the count to stabilize for
# 2 seconds to ensure all events from the current batch are processed.
PREV_SLEEP_COUNT=0
wait_for_docuum() {
  echo 'Waiting for Docuum to go quiet…'
  local count new_count
  # Wait for at least one new "Going back to sleep" message.
  while true; do
    count=$(grep -c 'Going back to sleep' "$DOCUUM_LOG" 2>/dev/null || echo 0)
    [[ "$count" -gt "$PREV_SLEEP_COUNT" ]] && break
    sleep 0.5
  done
  # Wait for the count to stop increasing (quiescence).
  while true; do
    sleep 2
    new_count=$(grep -c 'Going back to sleep' "$DOCUUM_LOG" 2>/dev/null || echo 0)
    [[ "$new_count" -eq "$count" ]] && break
    count="$new_count"
  done
  PREV_SLEEP_COUNT="$count"
}

wait_for_startup

# This image uses ~5.5 MB.
echo "Using an image we don't want to delete…"
docker container run --rm alpine@sha256:f27cad9117495d32d067133afff942cb2dc745dfe9163e949f6bfe8a6a245339 \
  true
docker image tag alpine@sha256:f27cad9117495d32d067133afff942cb2dc745dfe9163e949f6bfe8a6a245339 \
  alpine:keep

wait_for_docuum

# This image also uses ~5.5 MB.
echo 'Using another image…'
docker container run --rm alpine@sha256:2039be0c5ec6ce8566809626a252c930216a92109c043f282504accb5ee3c0c6 true

wait_for_docuum

# This image also uses ~5.5 MB. For some reason, this pushes us over the 20 MB
# threshold, even though we've only downloaded ~5.5 MB * 3 = ~16.5 MB.
echo 'Using another image…'
docker container run --rm alpine@sha256:4d889c14e7d5a73929ab00be2ef8ff22437e7cbc545931e52554a7b00e123d8b true

wait_for_docuum

# Assert that the image protected by the `--keep` flag is still present.
echo 'Checking that the protected image is still present…'
docker image inspect alpine@sha256:f27cad9117495d32d067133afff942cb2dc745dfe9163e949f6bfe8a6a245339 > \
  /dev/null 2>&1

# Assert that the last image is still present.
echo 'Checking that the last image is still present…'
docker image inspect alpine@sha256:4d889c14e7d5a73929ab00be2ef8ff22437e7cbc545931e52554a7b00e123d8b > \
  /dev/null 2>&1

# Assert that the first non-protected image was deleted.
echo 'Checking that the first non-protected image was deleted…'
if docker image inspect alpine@sha256:2039be0c5ec6ce8566809626a252c930216a92109c043f282504accb5ee3c0c6 \
  > /dev/null 2>&1
then
  echo "The image wasn't deleted."
  exit 1
fi

# Kill Docuum.
echo 'Killing Docuum…'
kill "$DOCUUM_PID"
