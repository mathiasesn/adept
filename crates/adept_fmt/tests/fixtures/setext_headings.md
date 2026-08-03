---
name: setext-headings-fixture
description: exercises setext heading round-tripping for adept_fmt tests.
---
Top Setext
==========

some text

Sub Setext
----------

more text

### Atx H3

done

## > quoted heading

Text whose leading word is block-marker-like must not be emitted in setext
form: reparsed, `> quoted heading` on its own line would read as a
blockquote, not a heading.

## 1. numbered heading

Same hazard, ordered-list-marker shaped.

## - dashed heading

Same hazard, bullet-marker shaped.

## # hash heading

Same hazard, a bare `#` run.

## >quoted heading

CommonMark's blockquote marker does not require a following space, so
`marker_like` (tuned for wrapped tokens, not setext line starts) would miss
this one; the heading printer's own allowlist must still reject it.

## >> nested quote heading

Same hazard, doubly so.

## ~~~fence

`marker_like`'s `~` arm only matches a *pure* run of tildes, so
`~~~fence` slips past it; must still fall back to ATX.

## ```fence

A leading backtick fence is the same code-fence hazard as `~~~`.

## |table| heading

Leading `|` reads as a table row.

## <div>hi heading

Leading `<div` reads as an HTML block start.

## [label]: dest heading

Leading `[label]:` reads as a link-reference definition.

## 多字节 heading

A multi-byte, non-ASCII-alphabetic-but-alphabetic first character is safe
and must stay setext.
