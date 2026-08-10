Guarantee IMAP session cleanup in the email reader. (@houko)
The helper previously logged out only on selected success and handled-error paths, leaking the connection when an unexpected exception occurred after login.
It now closes every constructed IMAP session through a non-masking cleanup path, including login failures and all post-login exits.
