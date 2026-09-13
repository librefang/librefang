Typing a negative penalty into a model's settings no longer saves it positive.
Entering `-0.25` by hand stored `+0.25`: the drawer keeps the parsed number, and `-0` — a legitimate value on the way to `-0.25` — reads back as `0`, which rewrote the field and erased the minus sign mid-keystroke.
Every negative fraction between -1 and 0 was affected except the `-0.5` preset, and nothing about the interface suggested the value had changed. (#8319) (@DaBlitzStein)
