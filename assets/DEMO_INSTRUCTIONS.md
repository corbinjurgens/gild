# Recording a demo

## Option 1: Screenshot (quickest)

1. Run gild against a repo with all add-ons:
   ```sh
   gild /path/to/repo --add-on ownership --add-on coupling --add-on hotspot --add-on authors --add-on types
   ```
2. Resize terminal to ~120x35 (a nice wide view)
3. Sort by impact (`i` key) so the table looks interesting
4. Take a screenshot (Cmd+Shift+4 on macOS, select the terminal window)
5. Save as `assets/demo.png`

## Option 2: GIF via VHS (best result)

Requires: `brew install charmbracelet/tap/vhs`

```sh
vhs assets/demo.tape
```

This produces `assets/demo.gif` automatically. Edit `demo.tape` to adjust timing.
If using GIF, update README.md to reference `demo.gif` instead of `demo.png`.

## Option 3: asciinema + agg

```sh
asciinema rec assets/demo.cast
# interact with gild, then exit
agg assets/demo.cast assets/demo.gif
```

Requires: `brew install asciinema` and `cargo install agg`
