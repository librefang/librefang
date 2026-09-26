A ClawHub route had no deadline of its own, so the client's retry policy was the only bound on how long a handler could take.
A hub that accepted the connection and then said nothing held `GET /api/clawhub/browse` for up to 270 s, against the 10 s every GET is given by the route smoke test, so a slow hub read as a broken route instead of the 503 this family already returns when a marketplace is unreachable.
The nine network round trips in the family now run under a budget of their own (#8453) (@DaBlitzStein)
