# Oxidize

A small CLI that checks whether your Oxide/uMod Rust plugins (`.cs`) actually compile — before you push them to a live server.

## Install

Download `oxidize-windows.exe` (or `oxidize-linux`) from the [latest release](https://github.com/ItzPabz/oxidize/releases/latest).

## Requirements

- [.NET 10 runtime](https://dotnet.microsoft.com/download)
- [DepotDownloader](https://github.com/SteamRE/DepotDownloader/releases) placed in Oxidize's `tools` folder (it prints the exact path if it's missing)

On first run, Oxidize downloads the Rust server libraries, Oxide, and its compiler automatically.

## Usage

```sh
oxidize <PATH>
```

`<PATH>` is a single plugin file or a folder of plugins.

| Flag | Description |
|------|-------------|
| `-o, --output <human\|json>` | Output format (default: `human`) |
| `-s, --staging` | Check against the Rust staging branch |

## Example

```sh
oxidize ./plugins
```
```
OK    Vanish
FAIL  Custom Craft Times (3 errors)
        error CS0117: 'MemoryExtensions' does not contain ...
```
