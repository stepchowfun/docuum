#!/usr/bin/env bash
set -euxo pipefail

# See [ref:integration_test_step] for how to run this integration test.

# Wait for the Docker daemon to start up.
echo 'Waiting for Docker to start…'
while ! docker container ls > /dev/null 2>&1; do
  sleep 1
done

# Start Docuum in the background.
echo 'Starting Docuum…'
LOG_LEVEL=debug /docuum-x86_64-unknown-linux-musl --threshold '20 MB' --keep 'alpine:keep' &
DOCUUM_PID="$!"

# This function waits for Docuum to start sleeping by checking the process state. The process could
# be sleeping for many reasons, so this function may return prematurely. In that case, the
# integration test may fail flakily. To make that outcome less likely, we repeat this procedure
# several times at a 1 second interval.
wait_for_docuum() {
  echo 'Waiting for Docuum to sleep…'
  for _ in {0..7}; do
    sleep 1
    while [[ "$(awk '{ print $3 }' /proc/$DOCUUM_PID/stat)" != 'S' ]]; do
      sleep 1
    done
  done
}

wait_for_docuum

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
