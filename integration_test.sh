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
/artifacts/docuum-x86_64-unknown-linux-musl --threshold 13MB &
DOCUUM_PID="$!"

# This function waits for Docuum to start sleeping by checking the process state. The process could
# be sleeping for many reasons, so this function may return prematurely. In that case, the
# integration test will fail flakily.
wait_for_docuum() {
  echo 'Waiting for Docuum to sleep…'
  while [[ "$(awk '{ print $3 }' /proc/$DOCUUM_PID/stat)" != 'S' ]]; do
    sleep 1
  done
}

wait_for_docuum

# This image uses ~5.5 MB.
echo 'Using an image…'
docker run alpine@sha256:4716d67546215299bf023fd80cc9d7e67f4bdc006a360727fd0b0b44512c45db true

# This image also uses ~5.5 MB.
echo 'Using another image…'
docker run alpine@sha256:fef20cf0221c5c0eaae2d8f59081b07cd351a94ac83cdc74109b17ec90ce0a82 true

# This image also uses ~5.5 MB. Now we should be above the 13 MB threshold.
echo 'Using a third image…'
docker run alpine@sha256:6b987122c635cd4bf46e52d85bca765732c7a224866501742c549ccc852f8c53 true

wait_for_docuum

# Assert the two most recently used images are still present.
echo 'Checking that the last two images are still present…'
docker inspect alpine@sha256:6b987122c635cd4bf46e52d85bca765732c7a224866501742c549ccc852f8c53 > \
  /dev/null 2>&1
docker inspect alpine@sha256:fef20cf0221c5c0eaae2d8f59081b07cd351a94ac83cdc74109b17ec90ce0a82 > \
  /dev/null 2>&1

# Assert the first image was deleted.
echo 'Checking that the first image was deleted…'
if docker inspect alpine@sha256:4716d67546215299bf023fd80cc9d7e67f4bdc006a360727fd0b0b44512c45db \
  > /dev/null 2>&1
then
  echo "The image wasn't deleted."
  exit 1
fi

# Kill Docuum.
echo 'Killing Docuum…'
kill "$DOCUUM_PID"
