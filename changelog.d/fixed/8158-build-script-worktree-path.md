The placeholder is declared as a Cargo build input, so its absence forces the build script to run when another checkout shares the target directory.
A cached `librefang-api` build script now creates the embedded-dashboard placeholder in the worktree Cargo is currently building.
The prior compile-time manifest path remained baked into a reused build-script binary, so sharing a target directory could create `static/react` in a sibling worktree and leave a fresh checkout failing inside `include_dir!`. (#8210) (@be-student)
