#!/bin/sh

for i in autoconf; do
    echo "$i"
    $i
    if [ $? -ne 0 ]; then
	echo "Error $? in $i"
	exit 1
    fi
done

echo "./configure --enable-autogen $@"
LDFLAGS="-L/home/ewan/bytehound_ewan15/target/debug" LIBS="-lpreload_syscallee" ./configure --enable-autogen $@
if [ $? -ne 0 ]; then
    echo "Error $? in ./configure"
    exit 1
fi
