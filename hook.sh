#!/usr/bin/env bash

sed -i "s#version = \"${PREV_VERSION}\"#version = \"${NEW_VERSION}\"#g" ./default.nix
sed -i "s#${CRATE_NAME}/v${PREV_VERSION}#${CRATE_NAME}/v${NEW_VERSION}#g" ./default.nix
