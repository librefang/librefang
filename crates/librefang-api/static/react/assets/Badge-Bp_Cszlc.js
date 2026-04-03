import{j as e}from"./router-BjNmg7Pr.js";const a={default:"bg-main text-text-dim border-border-subtle",success:"bg-success/10 text-success border-success/20",warning:"bg-warning/10 text-warning border-warning/20",error:"bg-error/10 text-error border-error/20",info:"bg-info/10 text-info border-info/20",brand:"bg-brand/10 text-brand border-brand/20"},b={default:"bg-text-dim/40",success:"bg-success",warning:"bg-warning",error:"bg-error",info:"bg-info",brand:"bg-brand"};function i({className:n="",variant:r="default",dot:s=!1,children:t,...o}){return e.jsxs("span",{className:`
        inline-flex items-center gap-1.5 rounded-lg px-2 py-0.5
        text-[10px] font-black uppercase tracking-wider
        border transition-colors duration-200 whitespace-nowrap
        ${a[r]}
        ${n}
      `,...o,children:[s&&e.jsx("span",{className:`w-1.5 h-1.5 rounded-full ${b[r]}`}),t]})}export{i as B};
