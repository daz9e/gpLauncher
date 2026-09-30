<script lang="ts">
  import Icon from './Icon.svelte';

  let {
    value = $bindable(''),
    placeholder = '',
    icon,
    oninput,
    onkeydown,
    input = $bindable(),
    autofocus = false,
    label,
  }: {
    value?: string;
    placeholder?: string;
    icon?: string;
    oninput?: (value: string) => void;
    onkeydown?: (e: KeyboardEvent) => void;
    input?: HTMLInputElement;
    autofocus?: boolean;
    label?: string;
  } = $props();

  $effect(() => {
    if (autofocus) input?.focus();
  });
</script>

<div class="field" class:with-icon={icon}>
  {#if icon}<span class="field-icon"><Icon name={icon} size={13} /></span>{/if}
  <input
    bind:this={input}
    bind:value
    {placeholder}
    aria-label={label ?? placeholder}
    spellcheck="false"
    autocomplete="off"
    oninput={() => oninput?.(value)}
    {onkeydown}
  />
</div>
