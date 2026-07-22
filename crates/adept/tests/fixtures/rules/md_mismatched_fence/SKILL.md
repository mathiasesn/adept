---
name: md-mismatched-fence
description: Use when the user asks to test markdown fence lexing where a code fence is closed by the wrong fence character. Do not use for prose skills.
---
# Mismatched Fence

```text
# Not A Heading
[broken](does/not/exist.md)
~~~
### Still Inside The First Fence
```

~~~text
# Also Not A Heading
[broken](also/missing.md)
```
##### Still Inside The Second Fence
~~~
