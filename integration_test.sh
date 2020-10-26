#!/bin/bash
set -Eemuo pipefail
# This script is meant to be run inside https://hub.docker.com/_/docker
# See [ref:integration_test_step] for how to run this yourself.

# Wait for the Docker daemon to start up.
for i in {1..30}; do
    if docker ps; then
        break
    else
        sleep 1
    fi
done
if [ "$i" = '30' ]; then
    echo "Docker did not start within 30 seconds. Aborting…"
    exit 1
fi

# Start Docuum in the background.
/artifacts/docuum-x86_64-unknown-linux-musl --threshold=13MB &

# Wait for docuum to start sleeping by checking the process state
function wait_for_docuum {
    # Explicitly sleep to prevent `docker exec` crashing while reading /proc
    # sleep 1
    # http://man7.org/linux/man-pages/man5/proc.5.html
    for i in {1..60}; do
        if [[ "$(awk '{ print $3 }' /proc/$!/stat)" = 'S' ]]; then
            break
        fi
        echo "Waiting for Docuum to finish one loop…"
        sleep 1
    done
    if [[ "$i" = '60' ]]; then
        echo 'Docuum did not finish a loop within 60 seconds. Aborting…' >&2
        exit 1
    fi
}

wait_for_docuum

# ~5.5 MB
docker run alpine@sha256:4716d67546215299bf023fd80cc9d7e67f4bdc006a360727fd0b0b44512c45db true
# ~5.5 MB
docker run alpine@sha256:fef20cf0221c5c0eaae2d8f59081b07cd351a94ac83cdc74109b17ec90ce0a82 true
# ~5.5 MB (now more than the 13 MB threshold)
docker run alpine@sha256:6b987122c635cd4bf46e52d85bca765732c7a224866501742c549ccc852f8c53 true

wait_for_docuum

# Assert the two most-recently-used alpine images are still present
docker inspect alpine@sha256:6b987122c635cd4bf46e52d85bca765732c7a224866501742c549ccc852f8c53 > /dev/null
docker inspect alpine@sha256:fef20cf0221c5c0eaae2d8f59081b07cd351a94ac83cdc74109b17ec90ce0a82 > /dev/null
# Assert the first alpine image was removed
! docker inspect alpine@sha256:4716d67546215299bf023fd80cc9d7e67f4bdc006a360727fd0b0b44512c45db > /dev/null 2>&1 || { echo "The first alpine image should have been deleted." >&2; false; }
kill $!

echo "Integration test passed!"
