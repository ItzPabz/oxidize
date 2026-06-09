# Oxidize

A CLI that checks whether your Oxide Rust plugins compile, before you put them to a live server.

## AI Use Disclosure

I used AI for two parts of this project:

- The C# compiler bridge in [`compiler/Bridge.cs`](compiler/Bridge.cs)
- The `compile_all` function in [`src/main.rs`](src/main.rs#L359)

I plan to write these by hand eventually, but I want to be honest about AI use in this project!

## Install

Download `oxidize-win.exe` (or `oxidize-linux`) from the [latest release](https://github.com/ItzPabz/oxidize/releases/latest).

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
OK    Working Plugin by Me
FAIL  Broken Plugin by You (3 errors)
        error CS0117: 'MemoryExtensions' does not contain ...
```
