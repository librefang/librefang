`main` went red on a goals test that was racing the goal runner rather than testing it.
The run it starts is deliberately pointed at an agent id that resolves to nothing, so the runner's first turn fails and the run ends on its own — after the assertions on an idle machine, before them on a loaded CI shard.
It now asserts that the run's registry entry survives the edit, which is what "an ordinary edit must not stop the run" actually means and the only signal that tells a stopped run apart from one that ended by itself. (#8362) (@DaBlitzStein)
