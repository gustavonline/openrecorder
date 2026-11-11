// Sets the colour theme before first paint (a blocking, same-origin script: the page's policy allows no inline script).
try{const t=localStorage.getItem("openrecorder-site-theme");document.documentElement.dataset.theme=["dark","light"].includes(t)?t:(matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light")}catch{document.documentElement.dataset.theme="light"}
