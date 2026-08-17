Reject PDF payloads above 20 MiB and stream extracted characters into a 200K-character sink so output truncation no longer requires first allocating the complete text. (#7566) (@houko)
