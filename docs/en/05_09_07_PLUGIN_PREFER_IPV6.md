 # Prefer IPv6 Plugin

 `prefer_ipv6` removes IPv4 answers (A records) from both the answer and additional sections of a response, leaving only IPv6 (AAAA) records when present.

 ## Exec quick-setup

 - Quick: `prefer_ipv6` (use as `exec: prefer_ipv6` to strip A records from the current response).

 ## Args

 - This plugin takes no arguments; use the exact quick-setup string `prefer_ipv6`.

 ## Behavior

 - When executed, if a response exists, `prefer_ipv6` will:
   - remove any `A` records from the response answers,
   - remove any `A` records from the response additional section.
 - If there is no response, the plugin does nothing.

 ## When to use

 - Prefer IPv6-only answers for clients or networks that prefer IPv6.
 - Bias responses toward AAAA records when both A and AAAA are present.

 ## Examples

 - Strip A records from upstream responses:

 ```
 - exec: forward
 - exec: prefer_ipv6
 ```

 - Use inside a sequence to normalize responses before caching or further processing.
