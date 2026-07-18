## Multibuffers

A multibuffer is a buffer made from excerpts across one or more files. Use one
when a task spans many locations, such as search results, diagnostics, symbols,
or a set of selected ranges.

### Opening a multibuffer

In pickers that point to locations in files, press `Ctrl-o` to open the current
matches as a multibuffer. This works in project search, diagnostics, and symbol
pickers.

For example:

1. Open project search with `Space-/`.
2. Search for a pattern.
3. Press `Ctrl-o`.

Helix opens a multibuffer with one excerpt per matching location. Each excerpt
includes the matching line and nearby context. Overlapping excerpts from the same
file are merged.

You can also create a multibuffer from the current selections with:

```helix
:open_selections_in_multibuffer
```

This is useful after splitting selections with commands such as `s` or `S`, then
continuing the edit in a smaller buffer.

### Editing and saving

Inside a multibuffer, normal movement and editing commands work on excerpts.
Saving the multibuffer with `:w` writes changed excerpts back to their source
files.

If a source excerpt changed outside the multibuffer since it was opened, `:w`
will fail rather than overwrite those changes. Use `:w!` to force the write.

Changing the shape of the projection, such as expanding an excerpt, does not by
itself make the multibuffer modified.

### Working with excerpts

The default keybinding `Shift-Enter` expands the excerpt under the cursor by a
few lines in both directions.

The following commands are also available:

| Command | Description |
| --- | --- |
| `:extend_multibuffer_excerpt` | Expand the current excerpt upward and downward |
| `:expand_multibuffer_excerpt_up` | Expand the current excerpt upward |
| `:expand_multibuffer_excerpt_down` | Expand the current excerpt downward |
| `:goto_next_multibuffer_excerpt` | Move to the start of the next excerpt |
| `:goto_previous_multibuffer_excerpt_end` | Move to the end of the previous excerpt |
| `:open_multibuffer_excerpt_vsplit` | Open the current excerpt's source in a vertical split |

When expanding an excerpt makes it overlap or touch another excerpt from the same
file, Helix merges them into a single excerpt.
