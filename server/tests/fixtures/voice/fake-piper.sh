#!/bin/sh
# Stands in for piper in tests: reads the text, writes one 16-bit sample per character.
# Only --output-raw is supported, which is how the server calls it.
text=$(cat)
n=$(printf '%s' "$text" | wc -c)
i=0
while [ "$i" -lt "$n" ]; do
    printf '\001\000'
    i=$((i + 1))
done
