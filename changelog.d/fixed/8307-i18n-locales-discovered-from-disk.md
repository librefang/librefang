The CLI locale coverage test now discovers the locales it checks by reading `crates/librefang-cli/locales/`, instead of running against a hand-written list.
The list was how `ko` went uncovered while shipping a complete 2510-line locale, and a hand-written list re-arms that for whichever locale is added next — the test cannot fail for a locale nobody remembered to add to it.
Its failure message now names the file (`locales/<code>/main.ftl`) and the count, so a locale added without translations says which one it is. (#8307) (@houko)
