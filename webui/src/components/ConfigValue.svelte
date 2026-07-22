<script lang="ts">
  import { darkMode } from "../lib/stores";

  // The value to render (any JSON value).
  export let value: unknown;
  // Current nesting depth; used to stop runaway recursion.
  export let depth = 0;

  // Max recursion depth before falling back to a JSON string, so a maliciously
  // or accidentally deep object can never blow the stack or the DOM.
  const MAX_DEPTH = 6;
  // Values longer than this (chars) are truncated with an ellipsis and reveal
  // the full text on hover via the native `title` attribute.
  const TRUNCATE_AT = 60;

  // True when a scalar string is long enough to need truncation.
  $: needsTruncate =
    typeof value === "string" && value.length > TRUNCATE_AT;
  $: truncated =
    typeof value === "string"
      ? value.slice(0, TRUNCATE_AT) + "…"
      : "";
</script>

{#if depth >= MAX_DEPTH}
  <span class={$darkMode ? "text-gray-500 italic" : "text-gray-400 italic"}>
    …
  </span>
{:else if value === null}
  <span class={$darkMode ? "text-gray-500" : "text-gray-400"}>null</span>
{:else if typeof value === "boolean"}
  <span class={$darkMode ? "text-purple-400" : "text-purple-600"}>{value}</span>
{:else if typeof value === "number"}
  <span class={$darkMode ? "text-blue-400" : "text-blue-600"}>{value}</span>
{:else if typeof value === "string"}
  <span
    class="font-mono text-xs {$darkMode ? 'text-green-400' : 'text-green-700'} break-all"
    title={needsTruncate ? value : undefined}
  >
    {needsTruncate ? truncated : value}
  </span>
{:else if Array.isArray(value)}
  {#if value.length === 0}
    <span class={$darkMode ? "text-gray-500" : "text-gray-400"}>[]</span>
  {:else}
    <ol class="space-y-1 mt-1">
      {#each value as item, i}
        <li class="flex items-start gap-2">
          <span
            class="flex-shrink-0 text-xs font-mono {$darkMode
              ? 'text-gray-500'
              : 'text-gray-400'} mt-0.5 w-6 text-right"
          >
            {i + 1}.
          </span>
          <div class="flex-1 min-w-0">
            <svelte:self value={item} depth={depth + 1} />
          </div>
        </li>
      {/each}
    </ol>
  {/if}
{:else if typeof value === "object" && value !== undefined}
  {@const entries = Object.entries(value as Record<string, unknown>)}
  {#if entries.length === 0}
    <span class={$darkMode ? "text-gray-500" : "text-gray-400"}>{"{}"}</span>
  {:else}
    <dl
      class="space-y-1 mt-1 {depth > 0
        ? 'border-l ' +
          ($darkMode ? 'border-gray-600' : 'border-gray-300') +
          ' pl-3 ml-1'
        : ''}"
    >
      {#each entries as [key, val]}
        <div class="flex items-start gap-2">
          <dt
            class="flex-shrink-0 text-xs font-mono font-medium {$darkMode
              ? 'text-gray-300'
              : 'text-gray-700'}"
          >
            {key}:
          </dt>
          <dd class="flex-1 min-w-0">
            <svelte:self value={val} depth={depth + 1} />
          </dd>
        </div>
      {/each}
    </dl>
  {/if}
{/if}
