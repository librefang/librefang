A media provider this daemon has no way to reach no longer appears in the provider list with an empty capability array.
  It filed under no dashboard tab and read as "this provider can do nothing" rather than "we have no way to serve this yet" — reachable without a code change, since the provider registry is fetched rather than checked in.
  The two `MediaDriverCache` constructors also stop spelling the built-in driver names inline, which is the duplication `BUILTIN_MEDIA_DRIVERS` was added to remove and which its doc comment already claimed was gone. (#8344) (@houko)
