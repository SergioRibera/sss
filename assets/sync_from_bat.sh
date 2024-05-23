#!/usr/bin/env bash

if [[ -d ../../bat ]]; then
  git -C ../../bat pull
else
  git clone --depth 1 --recurse-submodules https://github.com/sharkdp/bat ../../bat
  if [[ -d ../../bat_themes ]]; then
    git -C ../../bat_themes pull
  else
    git clone --depth 1 --recurse-submodules https://github.com/NatProgramer/bat_themes ../../bat_themes
  fi
fi

rm -rf ./syntaxes/* ./themes/*
cp -r ../../bat/assets/syntaxes/* ./syntaxes/
cp -r ../../bat/assets/themes/* ./themes/
<<<<<<< HEAD
cp -r ../../bat_themes/* ./themes
=======
cp -r ../../bat_themes/themes/* ./themes
>>>>>>> e373941 (chore: add extra themes to sync bat themes script)

cargo run -p sss_code -- --build-cache . -o .

echo "Finished."
