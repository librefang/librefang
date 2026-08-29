Cloning an agent whose `.identity` had been replaced by a file no longer reports a complete clone on Windows.
The check relied on `Path::try_exists()` returning an error for a non-directory path component, which is Unix behaviour; Windows reduces the same condition to "does not exist", so the clone silently fell back to the pre-migration workspace-root identity files instead of flagging the failure.
(#7961) (@houko)
