Creating a goal with a blank `evaluator_model` no longer stores an empty string where updating the same goal would have removed the field.
Update treats a blank value as the signal to clear the key, and the goal runner filters it again on read, so nothing downstream misbehaved — but a created goal round-tripped differently from an updated one and `GET /api/goals` handed the dashboard a value no update would ever have written.
Create now drops a blank one the way its siblings do (#7785) (@DaBlitzStein)
