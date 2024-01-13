# **S**uper **S**creen**S**hot
It is a set of libraries and tools for building screenshots in a standardized, high-performance image format.

| SSS Code                                                                                         | SSS Screenshot          |
|--------------------------------------------------------------------------------------------------|-------------------------|
| ![out](https://github.com/SergioRibera/sss/assets/56278796/be74cd48-8f87-4544-98da-c7bc119753ab) | ![out](https://github.com/SergioRibera/sss/assets/56278796/945f224c-96ec-48b6-a738-50ac2c9cfb90) |

## Libraries
- [sss_lib](./crates/sss_lib): Base library for screenshot generation
- [sss_code](./crates/sss_code): Terminal tool to take screenshot of your code
- [sss](./crates/sss_cli): Terminal tool to take screenshot of your screen

## Installation
> [!IMPORTANT]
> For the moment it is not published in any store so you have to download the tool from the [releases](https://github.com/SergioRibera/sss/releases)

## Acknowledgments
- [syntect](https://github.com/trishume/syntect): Rust library for syntax highlighting using Sublime Text syntax definitions. 
  - I use this library for code highlighting and parsing.
- [djanho](https://github.com/viniciusmuller/djanho): Convert VSCode themes to (Neo)Vim colorschemes
  - Use this project to understand how to import (Neo)vim themes.

## Other Goals
- [silicon](https://github.com/Aloxaf/silicon): Create beautiful image of your source code. 
  - I used it as a basis for my code screenshot project.
