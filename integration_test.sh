#!/usr/bin/env bash
set -euo pipefail

# See [ref:integration_test_step] for how to run this integration test.

# Wait for the Docker daemon to start up.
echo 'Waiting for Docker to start…'
while ! docker ps > /dev/null 2>&1; do
  sleep 1
done

# Start Docuum in the background.
echo 'Starting Docuum…'
/docuum-x86_64-unknown-linux-musl --threshold '20 MB' --keep 'alpine:keep' &
DOCUUM_PID="$!"

# This function waits for Docuum to start sleeping by checking the process state. The process could
# be sleeping for many reasons, so this function may return prematurely. In that case, the
# integration test will fail flakily. To make that outcome less likely, we sleep for 10 seconds
# before checking the process state.
wait_for_docuum() {
  echo 'Waiting for Docuum to sleep…'
  sleep 10
  while [[ "$(awk '{ print $3 }' /proc/$DOCUUM_PID/stat)" != 'S' ]]; do
    sleep 1
  done
}

wait_for_docuum

# This image uses ~5.5 MB.
echo "Using an image we don't want to delete…"
docker run alpine@sha256:52a197664c8ed0b4be6d3b8372f1d21f3204822ba432583644c9ce07f7d6448f \
  true
docker tag alpine@sha256:52a197664c8ed0b4be6d3b8372f1d21f3204822ba432583644c9ce07f7d6448f \
  alpine:keep

# This image also uses ~5.5 MB.
echo 'Using another image…'
docker run alpine@sha256:4716d67546215299bf023fd80cc9d7e67f4bdc006a360727fd0b0b44512c45db true

# This image also uses ~5.5 MB.
echo 'Using another image…'
docker run alpine@sha256:fef20cf0221c5c0eaae2d8f59081b07cd351a94ac83cdc74109b17ec90ce0a82 true

# This image also uses ~5.5 MB. Now we should be above the 20 MB threshold.
echo 'Using another image…'
docker run alpine@sha256:6b987122c635cd4bf46e52d85bca765732c7a224866501742c549ccc852f8c53 true

wait_for_docuum

# Assert that the image protected by the `--keep` flag is still present.
echo 'Checking that the protected image is still present…'
docker inspect alpine@sha256:52a197664c8ed0b4be6d3b8372f1d21f3204822ba432583644c9ce07f7d6448f > \
  /dev/null 2>&1

# Assert that the last two images are still present.
echo 'Checking that the last two images are still present…'
docker inspect alpine@sha256:fef20cf0221c5c0eaae2d8f59081b07cd351a94ac83cdc74109b17ec90ce0a82 > \
  /dev/null 2>&1
docker inspect alpine@sha256:6b987122c635cd4bf46e52d85bca765732c7a224866501742c549ccc852f8c53 > \
  /dev/null 2>&1

# Assert that the first non-protected image was deleted.
echo 'Checking that the first non-protected image was deleted…'
if docker inspect alpine@sha256:4716d67546215299bf023fd80cc9d7e67f4bdc006a360727fd0b0b44512c45db \
  > /dev/null 2>&1
then
  echo "The image wasn't deleted."
  exit 1
fi

# Kill Docuum.
echo 'Killing Docuum…'
kill "$DOCUUM_PID"
