# Keys

Every key runs a [command](commands.md). `?` shows them all, searchable by
what they do, the command, or the key (“left”, “ctrl z”).

Arrows, `Enter`, `Space`, `Escape` and Page Up/Down act on the calendar
only while it has focus, so they still work in the sidebar and dialogs.

## Changing them

Put bindings in `~/.config/rical-calendar/binds.conf` (or the file the
[`binds`](configuration.md#binds) setting names). They are layered over the
defaults below: a key you bind replaces the default, and a key bound to
nothing (`x =`) removes it. The format is aerc's:

```ini
# ~/.config/rical-calendar/binds.conf
[global]
<C-t> = :today<Enter>
gd = :goto<space>          # no <Enter>: opens the command line, typed so far
x =                        # unbind x

[month]
<Down> = :focus +7<Enter>
```

- A binding's value is what is typed: `:command args<Enter>` runs it;
  several can follow one another; without `<Enter>` the command line opens
  with the text in it, for you to finish.
- Keys are a character (`a`, `A`, `[`) or `<…>`: a name such as `<Enter>`,
  `<Esc>`, `<Space>`, `<Tab>`, `<Backspace>`, `<Delete>`, `<Up>`,
  `<PageDown>`, `<Home>`, `<F5>`, `<lt>` (for `<`), with `C-` (Ctrl), `A-`
  (Alt) or `S-` (Shift) in front: `<C-z>`, `<S-Down>`, `<A-1>`. `A` is
  Shift+a; `<C-A>` is Ctrl+Shift+a.
- A key sequence is keys one after another: `gd`, `<C-x>k`.
- `[global]` applies everywhere; a view's section (`[grid]` for the day and
  week views, `[month]`, `[year]`, `[agenda]`, `[tasks]`, `[invitations]`,
  `[journal]`) takes precedence in that view.

## The defaults

### `[global]`

| Key | Runs |
|---|---|
| `:` | `: …` (you type the rest) |
| `?` | `:help` |
| `/` | `:search` |
| `<C-q>` | `:quit` |
| `<C-r>` | `:reload` |
| `<F5>` | `:reload` |
| `<F9>` | `:sidebar` |
| `<C-,>` | `:settings` |
| `t` | `:today` |
| `n` | `:next` |
| `p` | `:prev` |
| `<PageDown>` | `:next` |
| `<PageUp>` | `:prev` |
| `g` | `:goto …` (you type the rest) |
| `<C-g>` | `:goto …` (you type the rest) |
| `h` | `:focus -1` |
| `l` | `:focus +1` |
| `<Left>` | `:focus -1` |
| `<Right>` | `:focus +1` |
| `d` | `:view day` |
| `w` | `:view week` |
| `m` | `:view month` |
| `z` | `:view year` |
| `a` | `:view agenda` |
| `T` | `:view tasks` |
| `I` | `:view invitations` |
| `E` | `:view journal` |
| `1` | `:view day` |
| `2` | `:view 2days` |
| `3` | `:view 3days` |
| `4` | `:view 4days` |
| `5` | `:view 5days` |
| `6` | `:view 6days` |
| `7` | `:view fullweek` |
| `W` | `:view workweek` |
| `^` | `:up` |
| `Z` | `:zones` |
| `X` | `:all-day` |
| `<C-1>` | `:view day` |
| `<C-2>` | `:view week` |
| `<C-3>` | `:view month` |
| `<C-4>` | `:view agenda` |
| `<C-5>` | `:view tasks` |
| `<C-6>` | `:view invitations` |
| `<C-7>` | `:view journal` |
| `j` | `:select next` |
| `k` | `:select prev` |
| `<Down>` | `:select next` |
| `<Up>` | `:select prev` |
| `<Enter>` | `:open` |
| `<space>` | `:open` |
| `<Esc>` | `:select none` |
| `c` | `:new` |
| `<C-n>` | `:new` |
| `C` | `:new …` (you type the rest) |
| `e` | `:edit` |
| `O` | `:link` |
| `M` | `:map` |
| `R` | `:raw` |
| `v` | `:reveal` |
| `<C-t>` | `:convert` |
| `<C-e>` | `:edit` |
| `x` | `:delete` |
| `<Delete>` | `:delete` |
| `J` | `:move +15m` |
| `K` | `:move -15m` |
| `H` | `:move -1d` |
| `L` | `:move +1d` |
| `<S-Down>` | `:move +15m` |
| `<S-Up>` | `:move -15m` |
| `<S-Left>` | `:move -1d` |
| `<S-Right>` | `:move +1d` |
| `]` | `:resize +15m` |
| `[` | `:resize -15m` |
| `}` | `:resize +1d` |
| `{` | `:resize -1d` |
| `D` | `:done` |
| `y` | `:accept` |
| `Y` | `:maybe` |
| `N` | `:decline` |
| `A` | `:task …` (you type the rest) |
| `S` | `:schedule …` (you type the rest) |
| `u` | `:undo` |
| `U` | `:redo` |
| `<C-z>` | `:undo` |
| `<C-Z>` | `:redo` |
| `<C-y>` | `:redo` |
| `<C-h>` | `:history` |
| `<A-1>` | `:calendar toggle 1` |
| `<A-2>` | `:calendar toggle 2` |
| `<A-3>` | `:calendar toggle 3` |
| `<A-4>` | `:calendar toggle 4` |
| `<A-5>` | `:calendar toggle 5` |
| `<A-6>` | `:calendar toggle 6` |
| `<A-7>` | `:calendar toggle 7` |
| `<A-8>` | `:calendar toggle 8` |
| `<A-9>` | `:calendar toggle 9` |

### `[month]`

| Key | Runs |
|---|---|
| `<Down>` | `:focus +7` |
| `<Up>` | `:focus -7` |
| `<S-Down>` | `:move +7d` |
| `<S-Up>` | `:move -7d` |

### `[agenda]`

| Key | Runs |
|---|---|
| `<Down>` | *unbound in this view* |
| `<Up>` | *unbound in this view* |
| `<Enter>` | *unbound in this view* |
| `<space>` | *unbound in this view* |

### `[tasks]`

| Key | Runs |
|---|---|
| `<Down>` | *unbound in this view* |
| `<Up>` | *unbound in this view* |
| `<space>` | `:done` |
| `<Enter>` | `:open` |
| `c` | `:task …` (you type the rest) |
| `C` | `:subtask …` (you type the rest) |
| `s` | `:schedule …` (you type the rest) |
| `<Esc>` | *unbound in this view* |

### `[invitations]`

| Key | Runs |
|---|---|
| `<Down>` | *unbound in this view* |
| `<Up>` | *unbound in this view* |
| `<Enter>` | *unbound in this view* |
| `<space>` | *unbound in this view* |

### `[journal]`

| Key | Runs |
|---|---|
| `c` | `:note …` (you type the rest) |
| `C` | `:note` |
| `<Down>` | *unbound in this view* |
| `<Up>` | *unbound in this view* |
| `<space>` | *unbound in this view* |

### `[year]`

| Key | Runs |
|---|---|
| `<Down>` | `:focus +7` |
| `<Up>` | `:focus -7` |
| `j` | `:focus +7` |
| `k` | `:focus -7` |
