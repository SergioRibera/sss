#!/usr/bin/env bash

if [[ -d ../../bat ]]; then
  git -C ../../bat pull
else
  git clone --depth 1 --recurse-submodules https://github.com/sharkdp/bat ../../bat
  if [[ -d ../../bat_catppuccin ]]; then
    git -C ../../bat_catppuccin pull
  else
    git clone --depth 1 --recurse-submodules https://github.com/catppuccin/bat ../../bat_catppuccin
  fi
fi

rm -rf ./syntaxes/* ./themes/*
cp -r ../../bat/assets/syntaxes/* ./syntaxes/
cp -r ../../bat/assets/themes/* ./themes/
cp -r ../../bat_catppuccin/themes/* ./themes

cargo run -p sss_code -- --build-cache . -o .

echo "Finished."
