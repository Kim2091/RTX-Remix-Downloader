# RTX-Remix-Downloader

A simple downloader for the latest RTX Remix and Remix Plus builds.

It offers stable releases and development builds (the latest successful CI build) for:
* RTX Remix from [NVIDIAGameWorks/rtx-remix](https://github.com/NVIDIAGameWorks/rtx-remix) and [NVIDIAGameWorks/dxvk-remix](https://github.com/NVIDIAGameWorks/dxvk-remix/): 32-bit (x86), 64-bit (x64) and ARM64 games
* Remix Plus from [RemixProjGroup/dxvk-remix](https://github.com/RemixProjGroup/dxvk-remix): 32-bit (x86) and 64-bit (x64) games

Only the build types a release actually ships are offered. 32-bit installs also get dxwrapper's d3d8 as `d3d8_off.dll` for DirectX 8 games.

 # [Download](https://github.com/Kim2091/RTX-Remix-Downloader/releases/latest/download/RTX.Remix.Downloader.exe)

## Installation
Simply move the .exe file to a folder or desktop and run it. When a new version of the downloader is out, it tells you at startup and can update itself. It will create a subdirectory called "remix" that contains all the files it downloaded.
Running it again replaces that install. Only files the downloader installed and nobody changed are deleted; anything you added or edited is kept in a "remix-old-..." folder next to it.

## Usage
Execute the script and follow the instructions shown in the window.
Once the script finishes you'll find a fully patched RTX Remix install in the Remix folder next to the .exe file, you can then follow the normal [installation guide for RTX Remix](https://github.com/NVIDIAGameWorks/rtx-remix/wiki/runtime-user-guide).

https://github.com/user-attachments/assets/b85be332-3463-4bb9-8b55-bae60f3166cb

## Support
Find us on Discord: [RTX Remix Showcase](https://discord.gg/rtxremix)

Rust version by [me](https://github.com/Kim2091)

Original PowerShell script version by [Kowlin](https://github.com/Kowlin) & [RuneStorm](https://github.com/RuneStorm)

**This Project is in no way directly affiliated with NVIDIA or the RTX Remix project.**
