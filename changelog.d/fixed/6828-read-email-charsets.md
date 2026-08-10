Decode email bodies with their declared MIME charset. (@houko)
The IMAP email helper previously forced UTF-8 for multipart plain text, HTML fallback, and non-multipart bodies, silently dropping bytes from common encodings such as ISO-8859-1 and GB2312.
Each body part now uses its `charset` parameter while retaining UTF-8 as the fallback for missing or unknown charset labels.
