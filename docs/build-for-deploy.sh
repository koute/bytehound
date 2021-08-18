#!/bin/sh

rm -Rf .cache
rm -Rf src/generated
mdbook clean
mdbook build
