<img src="https://i.ibb.co/p3ysZLp/SSS.png">

# Super ScreenShot

Super ScreenShot is a set of libraries and tools for building screenshots in a standardized, high-performance image format made in rust.

<img src="https://i.ibb.co/y8Lvcgx/Outputs.png">

## 🧩 Libraries
It provides different options depending on your needs.

- [sss_lib](./crates/sss_lib): The base library for screenshot generation, providing the core functionality for the other tools.
- [sss_code](./crates/sss_code): A terminal tool specifically designed to take screenshots of your code, making it easy to share and showcase your snippets.
- [sss](./crates/sss_cli): A versatile terminal tool that allows you to capture screenshots of your entire screen or specific regions.

## 🚀 Installation
> [!IMPORTANT]
> At the moment the tool is not available in any store. Please redirect to [releases](https://github.com/SergioRibera/sss/releases) section in order to download.

## ⚙️ Configuration
> [!NOTE]
> To know how to configure it from a file, you can review the [default.toml](./examples/default_config.toml) file.
> 
All cli arguments can be set from a single file in the path `~/.config/sss/config.toml`, right here you can place the configuration for `sss_code` and `sss`.

## 💻 Usage
You can find examples and options in the following links.
- [sss_cli](https://sergioribera.rustlang-es.org/sss/sss/): Screenshots of your screen.
- [sss_code](https://sergioribera.rustlang-es.org/sss/sss_code/): Screenshots of your code.


## 💡 Acknowledgments
- [syntect](https://github.com/trishume/syntect): Rust library for syntax highlighting using Sublime Text syntax definitions. 
  - I use this library for code highlighting and parsing.
- [djanho](https://github.com/viniciusmuller/djanho): Convert VSCode themes to (Neo)Vim colorschemes
  - Use this project to understand how to import (Neo)vim themes.

## 🏁 Other Goals
- [silicon](https://github.com/Aloxaf/silicon): Create beautiful image of your source code. 
  - I used it as a basis for my code screenshot project.
