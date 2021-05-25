#!/bin/sh

FRAME_COUNT=8192

echo "" > generated.inc
echo "frame_t FRAMES[] = {" >> generated.inc

for i in $(seq 0 $(($FRAME_COUNT-2))); do
    echo "    frame_n<$i>," >> generated.inc
done

echo "    frame_n<$(($FRAME_COUNT-1))>" >> generated.inc

echo "};" >> generated.inc
echo "size_t FRAME_COUNT = $FRAME_COUNT;" >> generated.inc
