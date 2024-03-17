#!/usr/bin/env bash

if [[ -d ../../bat ]]; then
  git -C ../../bat pull
else
  git clone --depth 1 --recurse-submodules https://github.com/sharkdp/bat ../../bat
fi

rm -rf ./syntaxes/* ./themes/*
cp -r ../../bat/assets/syntaxes/* ./syntaxes/
cp -r ../../bat/assets/themes/* ./themes/

cargo run -p sss_code -- --build-cache . -o .

echo "Finished."
