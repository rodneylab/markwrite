const themeToggleCheckbox=document.querySelector("#theme-toggle"),darkThemeButton=document.querySelector(".dark-theme-button"),lightThemeButton=document.querySelector(".light-theme-button"),themePreference=window.localStorage.getItem("markwrite-theme");themePreference&&themePreference==="dark"||themePreference==null&&window.matchMedia("(prefers-color-scheme: dark)").matches?(document.styleSheets[1].disabled=!0,document.querySelector("body").classList.add("dark-theme"),themeToggleCheckbox.checked=!0,darkThemeButton.disabled=!0):(document.styleSheets[0].disabled=!0,lightThemeButton.disabled=!0);function toggleTheme(){const e=lightThemeButton.disabled;document.styleSheets[0].disabled=!e,document.styleSheets[1].disabled=e,document.querySelector("body").classList.toggle("dark-theme"),darkThemeButton.disabled=e,lightThemeButton.disabled=!e,window.localStorage.setItem("markwrite-theme",e?"dark":"light")}themeToggleCheckbox.addEventListener("change",()=>{toggleTheme()}),darkThemeButton.addEventListener("click",()=>{toggleTheme(),themeToggleCheckbox.checked=!0}),lightThemeButton.addEventListener("click",()=>{toggleTheme(),themeToggleCheckbox.checked=!1});
