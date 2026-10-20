export default function WorldArt() {
  return (
    <svg className="world-art" viewBox="0 0 420 300" aria-hidden="true">
      <defs>
        <radialGradient id="planet">
          <stop offset="0" stopColor="#56634c" />
          <stop offset="0.7" stopColor="#343e33" />
          <stop offset="1" stopColor="#202b26" />
        </radialGradient>
        <pattern id="grid" width="22" height="22" patternUnits="userSpaceOnUse">
          <path
            d="M22 0H0V22"
            fill="none"
            stroke="#c2cbb9"
            strokeOpacity=".09"
            strokeWidth=".5"
          />
        </pattern>
        <clipPath id="planet-clip">
          <circle cx="232" cy="142" r="98" />
        </clipPath>
      </defs>
      <circle
        cx="232"
        cy="142"
        r="126"
        fill="none"
        stroke="#687262"
        strokeOpacity=".25"
        strokeDasharray="2 7"
      />
      <ellipse
        cx="232"
        cy="142"
        rx="174"
        ry="61"
        fill="none"
        stroke="#b6b99c"
        strokeOpacity=".18"
        transform="rotate(-26 232 142)"
      />
      <circle
        cx="232"
        cy="142"
        r="98"
        fill="url(#planet)"
        stroke="#89927a"
        strokeOpacity=".3"
      />
      <g
        clipPath="url(#planet-clip)"
        fill="none"
        stroke="#9aa58b"
        strokeOpacity=".28"
      >
        <path d="M128 87 160 100 182 71 209 83 226 62 259 70 273 102 306 102 339 128M125 106 159 121 186 93 213 108 237 86 261 97 266 126 296 123 338 150M127 131 162 140 188 116 218 129 239 112 250 141 280 150 305 146 335 168M130 155 163 164 189 142 217 156 239 138 249 164 280 177 310 170 330 190M142 183 171 187 191 166 219 183 242 170 257 191 283 199 307 193M162 209 192 194 222 210 243 195 267 215 288 219" />
        <circle cx="232" cy="142" r="98" fill="url(#grid)" stroke="none" />
      </g>
      <g stroke="#d4a75b" strokeWidth="1.5" fill="#d4a75b">
        <path d="M200 132h28v20h22v25h22" fill="none" />
        <rect x="196" y="128" width="8" height="8" rx="1" />
        <rect x="224" y="148" width="8" height="8" rx="1" />
        <rect x="268" y="173" width="8" height="8" rx="1" />
        <circle cx="250" cy="152" r="3" />
      </g>
      <path
        d="M272 177h48l20 25h35M200 132h-52l-23-29H67"
        fill="none"
        stroke="#a8aa94"
        strokeOpacity=".4"
      />
      <circle cx="81" cy="185" r="3" fill="#d4a75b" />
      <circle cx="365" cy="90" r="2" fill="#a4ad96" />
      <text
        x="341"
        y="218"
        fill="#a8ad9f"
        fontSize="8"
        letterSpacing="2"
        fontFamily="monospace"
      >
        NAUVIS
      </text>
      <text
        x="67"
        y="96"
        fill="#a8ad9f"
        fontSize="8"
        letterSpacing="2"
        fontFamily="monospace"
      >
        HOME SYSTEM
      </text>
    </svg>
  );
}
