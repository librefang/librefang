The registry's git fallback ran `git fetch` and `git clone` with no deadline, so a credential prompt on a process with no tty — or a transfer that had stopped — waited indefinitely instead of failing.
Measured cost: the clone held the shared cargo lock for about half an hour from inside a test, and because `flock` is not FIFO the three lanes queued behind it stopped with it.
The two network calls now carry `GIT_TERMINAL_PROMPT=0` and git's own low-speed abort, which bound the ways the call can hang without bounding how long a transfer may take (#8462) (@DaBlitzStein)
