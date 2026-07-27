 # Prefer IPv4 Plugin

 `prefer_ipv4` removes IPv6 answers (AAAA records) from both the answer and additional sections of a response, leaving only IPv4 (A) records when present.

 ## Exec quick-setup

 - Quick: `prefer_ipv4` (use as `exec: prefer_ipv4` to strip AAAA records from the current response).

 ## Args

 - This plugin takes no arguments; use the exact quick-setup string `prefer_ipv4`.

 ## Behavior

 - When executed, if a response exists, `prefer_ipv4` will:
   - remove any `AAAA` records from the response answers,
   - remove any `AAAA` records from the response additional section.
 - If there is no response, the plugin does nothing.

 ## When to use

 - Prefer IPv4-only answers for clients or networks with limited IPv6 support.
 - Reduce the size of responses by dropping AAAA records when you want to bias clients toward A records.

 ## Examples

 - Strip AAAA records from upstream responses:

 ```
 - exec: forward
 - exec: prefer_ipv4
 ```

 - Use inside a sequence to normalize responses before caching or further processing.
