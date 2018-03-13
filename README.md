`codeswitch` is a minimal utility to make it easy to switch between git
repositories. Assuming you keep all your repos in some sort of tree structure
under a directory like `~/Code` or `/srv/hacking`, `codeswitch` can scan the
directory for repositories and lookup specific codebases.

For example, to find project `foobar`:

```
$ codeswitch /code foobar
/code/github/foobar
```

A complementary bash integration source file is available [here](shell/bash),
which defines a function `code`. You can then quickly change directory to
`foobar` using `code foobar`.

The first run will scan the directory and build a cache in
`~/.cache/codeswitch`. Subsequent runs will use the cache. Cache misses
automatically trigger a rebuild (or you can also use `--rebuild`).

If there are multiple matches, `codeswitch` will report all matches:

```
$ code foobar
error: multiple matches found
   1  /code/github/qwer/foobar
   2  /code/github/asdf/foobar
   3  /code/github/zxcv/foobar
```

You can then be more specific by providing an index (`code foobar 2`), or a
string to match (`code foobar qwer`).

`codeswitch` has some fancy handling for symbolic links. For example, if
`code/gh` is a symlink to `github`, then `codeswitch` will automatically drop
`github` paths in favour their shorter `gh` equivalents.

This project started as a [toy program](https://en.wikipedia.org/wiki/Toy_program)
to learn Rust, but I now use it daily as a (much faster) replacement to its
shell version. That said, there are probably still many improvements and tweaks
that could be made to make it more idiomatic Rust. If anything jumps out at you,
please feel free to open an issue or a PR to help me learn!
