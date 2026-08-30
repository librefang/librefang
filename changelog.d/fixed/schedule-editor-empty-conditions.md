Saving the agent schedule panel's conditions editor while it is empty no longer clears a proactive schedule.
The textarea starts empty and is never seeded from the live schedule, so opening Edit and pressing Save sent `conditions: []`, wiped conditions the operator had never looked at, and reported it as a successful save.
An empty list is now refused the same way `saveCron` has always refused a blank cron expression; clearing conditions deliberately is still available by switching the schedule mode away from proactive. (@DaBlitzStein)
