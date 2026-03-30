#!/usr/bin/env python3
"""Read latest email from a given sender via IMAP, searching multiple folders.

Environment variables:
  EMAIL_USERNAME  - IMAP login username (required)
  EMAIL_PASSWORD  - IMAP login password (required)
  IMAP_SERVER     - IMAP server hostname (default: imap.gmail.com)
  IMAP_PORT       - IMAP server port (default: 993)

Non-Gmail servers will only match INBOX, Promotions, and Updates folders;
Gmail-specific folders like [Gmail]/All Mail are skipped gracefully.
"""
import imaplib
import email
import os
import re
import sys
from email.header import decode_header

IMAP_SERVER = os.environ.get("IMAP_SERVER", "imap.gmail.com")
try:
    IMAP_PORT = int(os.environ.get("IMAP_PORT", "993"))
except ValueError:
    print("ERROR: IMAP_PORT must be an integer", file=sys.stderr)
    sys.exit(1)


def decode_str(s, enc=None):
    if isinstance(s, bytes):
        return s.decode(enc or "utf-8", errors="ignore")
    return s or ""

def get_body(msg):
    """Extract plain text body from email message."""
    if msg.is_multipart():
        for part in msg.walk():
            ct = part.get_content_type()
            cd = str(part.get("Content-Disposition", ""))
            if ct == "text/plain" and "attachment" not in cd:
                payload = part.get_payload(decode=True)
                if payload:
                    return payload.decode("utf-8", errors="ignore")
        # Fallback to HTML if no plain text
        for part in msg.walk():
            ct = part.get_content_type()
            cd = str(part.get("Content-Disposition", ""))
            if ct == "text/html" and "attachment" not in cd:
                payload = part.get_payload(decode=True)
                if payload:
                    text = payload.decode("utf-8", errors="ignore")
                    text = re.sub(r'<[^>]+>', ' ', text)
                    text = re.sub(r'\s+', ' ', text).strip()
                    return text
    else:
        payload = msg.get_payload(decode=True)
        if payload:
            return payload.decode("utf-8", errors="ignore")
    return ""

def search_folder(mail, folder, sender):
    """Search a folder for emails from sender. Returns list of IDs."""
    try:
        result, _ = mail.select(folder, readonly=True)
        if result != "OK":
            return []
        if sender:
            _, msgs = mail.search(None, f'FROM "{sender}"')
        else:
            _, msgs = mail.search(None, "ALL")
        ids = msgs[0].split() if msgs[0] else []
        return ids
    except Exception as e:
        print(f"WARNING: failed to search {folder}: {e}", file=sys.stderr)
        return []

def main():
    sender = sys.argv[1] if len(sys.argv) > 1 else ""
    password = os.environ.get("EMAIL_PASSWORD", "")
    username = os.environ.get("EMAIL_USERNAME", "")

    if not username:
        print("ERROR: EMAIL_USERNAME not set", file=sys.stderr)
        sys.exit(1)

    if not password:
        print("ERROR: EMAIL_PASSWORD not set", file=sys.stderr)
        sys.exit(1)

    try:
        mail = imaplib.IMAP4_SSL(IMAP_SERVER, IMAP_PORT)
        mail.login(username, password)
    except Exception as e:
        print(f"ERROR: IMAP login failed: {e}")
        sys.exit(1)

    # Search across multiple folders
    folders_to_try = [
        "INBOX",
        '"[Gmail]/All Mail"',
        '"[Gmail]/Promotions"',
        '"[Gmail]/Updates"',
        "Promotions",
        "Updates",
    ]

    found_ids = []
    found_folder = None
    for folder in folders_to_try:
        ids = search_folder(mail, folder, sender)
        if ids:
            found_ids = ids
            found_folder = folder
            break

    if not found_ids:
        print(f"NO_EMAIL_FOUND (searched: INBOX, All Mail, Promotions, Updates)")
        mail.logout()
        sys.exit(0)

    print(f"Found {len(found_ids)} email(s) in {found_folder}")

    # Fetch the latest email
    try:
        _, data = mail.fetch(found_ids[-1], "(RFC822)")
        msg = email.message_from_bytes(data[0][1])
    except Exception as e:
        print(f"ERROR: fetch failed: {e}")
        mail.logout()
        sys.exit(1)

    subject_raw, enc = decode_header(msg["Subject"])[0]
    subject = decode_str(subject_raw, enc)
    body = get_body(msg)

    print(f"SUBJECT: {subject}")
    print(f"FROM: {msg['From']}")
    print(f"DATE: {msg['Date']}")
    print("---BODY---")
    print(body[:4000])

    mail.logout()

if __name__ == "__main__":
    main()
