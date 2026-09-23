<script lang="ts">
    import "../app.css";
</script>

<!--
@component
Root layout for every route. It imports the global stylesheet `app.css` (Tailwind and
daisyUI) and renders the current page through `<slot />`. It has no state, reads no
sessionStorage and makes no NATS calls; each page opens its own connection.
-->
<slot />
