#!/bin/bash
set -Eemo pipefail
# This script is meant to be run inside https://hub.docker.com/_/docker
# See .github/workflows/ci.yml:18 for how to run this yourself

# Wait for the docker daemon to start up
for i in {1..30}; do
    if docker ps; then
        break
    else
        sleep 1
    fi
done
if [ "$i" = '30' ]; then
    echo "Docker did not start within 30 seconds, aborting"
    exit 1
fi

# Start docuum in the background
/artifacts/docuum-x86_64-unknown-linux-musl --threshold=13MB &
sleep 1
# Wait for docuum to start sleeping by checking the process state
function wait {
    # http://man7.org/linux/man-pages/man5/proc.5.html
    DOCUUM_PID=$(pgrep docuum-x86_64-unknown-linux-musl)
    for i in {1..60}; do
        if [ "$(awk '{ print $3 }' /proc/${DOCUUM_PID}/stat)" = 'S' ]; then
            break
        fi
        echo "Waiting for docuum to finish one loop..."
        sleep 1
    done
    if [ "$i" = '60' ]; then
        echo "Docuum did not finish a loop within 1 minute, aborting"
        exit 1
    fi
}

wait

# ~5.5MB
docker run alpine@sha256:4716d67546215299bf023fd80cc9d7e67f4bdc006a360727fd0b0b44512c45db true
# ~5.5MB
docker run alpine@sha256:fef20cf0221c5c0eaae2d8f59081b07cd351a94ac83cdc74109b17ec90ce0a82 true
# ~5.5MB (now more than the 13MB threshold)
docker run alpine@sha256:6b987122c635cd4bf46e52d85bca765732c7a224866501742c549ccc852f8c53 true

wait

# Assert the two most-recently-used alpine images are still present
docker inspect alpine@sha256:6b987122c635cd4bf46e52d85bca765732c7a224866501742c549ccc852f8c53 > /dev/null
docker inspect alpine@sha256:fef20cf0221c5c0eaae2d8f59081b07cd351a94ac83cdc74109b17ec90ce0a82 > /dev/null
# Assert the first alpine image was removed
! docker inspect alpine@sha256:4716d67546215299bf023fd80cc9d7e67f4bdc006a360727fd0b0b44512c45db > /dev/null 2>&1 || { echo "The first alpine image should have been deleted."; false; }

kill $!

echo "Integration test passed!"
