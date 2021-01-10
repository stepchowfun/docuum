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
if $WINDOWS; then
  # Windows integration tests doesn't run with dind so we need to prune all images on the CI worker
  echo 'Pruning images on Windows CI worker…'
  docker image prune -a -f
  target/release/docuum.exe --threshold 725MB &
else
  /artifacts/docuum-x86_64-unknown-linux-musl --threshold 13MB &
fi
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

if $WINDOWS; then
  # This image uses ~347 MB.
  IMAGE1="mcr.microsoft.com/windows/nanoserver@sha256:4fbb09b6f685754391b86d86399d3a52f95ed0e6e5174947e205dd86df90cf3a"
  # This image uses ~252 MB.
  IMAGE2="mcr.microsoft.com/windows/nanoserver@sha256:5e8cbdb57a33156c9f37bec01b6add720532d1432ce4ade821920c4f7b8c6409"
  # This image also uses ~252 MB. Now we should be above the 725 MB threshold.
  IMAGE3="mcr.microsoft.com/windows/nanoserver@sha256:5f7004ad6559594da4e44e661f4feee58f291ed01ef71717a2041c40d45655d8"
else
  # This image uses ~5.5 MB.
  IMAGE1="alpine@sha256:4716d67546215299bf023fd80cc9d7e67f4bdc006a360727fd0b0b44512c45db"
  # This image also uses ~5.5 MB.
  IMAGE2="alpine@sha256:fef20cf0221c5c0eaae2d8f59081b07cd351a94ac83cdc74109b17ec90ce0a82"
  # This image also uses ~5.5 MB. Now we should be above the 13 MB threshold.
  IMAGE3="alpine@sha256:6b987122c635cd4bf46e52d85bca765732c7a224866501742c549ccc852f8c53"
fi


echo 'Using an image…'
docker run $IMAGE1

echo 'Using another image…'
docker run $IMAGE2

echo 'Using a third image…'
docker run $IMAGE3

wait_for_docuum

# Assert the two most recently used images are still present.
echo 'Checking that the last two images are still present…'
docker inspect $IMAGE3 > \
  /dev/null 2>&1
docker inspect $IMAGE2 > \
  /dev/null 2>&1

# Assert the first image was deleted.
echo 'Checking that the first image was deleted…'
if docker inspect $IMAGE1 \
  > /dev/null 2>&1
then
  echo "The image wasn't deleted."
  exit 1
fi

# Kill Docuum.
echo 'Killing Docuum…'
kill "$DOCUUM_PID"
