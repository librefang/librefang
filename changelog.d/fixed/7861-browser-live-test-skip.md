A live browser extraction fixture no longer fails the build when Chromium is installed but cannot start.
The helper already skipped the test when no Chromium binary was found, but panicked when a discovered binary failed to launch — which is the ordinary case on a CI runner with no sandbox, a missing shared library, or too little memory.
Because the test runs in the default lane, that turned an environment limitation into a red `main` that every open pull request then inherited on its next run.
A launch failure is now treated as the same class of unavailability as a missing binary, so the fixture skips with a message instead of aborting the lane.
(#7876) (@houko)
