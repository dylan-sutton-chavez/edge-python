'use client'

import { useEffect } from 'react'

// Switch to the hamburger when the nav links overflow instead of a fixed breakpoint.
export function NavbarAutoCollapse() {
    useEffect(() => {
        const nav = document.querySelector('.nextra-navbar nav')
        const links = nav?.querySelector('.nextra-scrollbar')
        if (!nav || !links) return

        const measure = () => {
            document.documentElement.removeAttribute('data-nav-cramped') // measure desktop layout first
            if (links.scrollWidth > links.clientWidth + 1)
                document.documentElement.setAttribute('data-nav-cramped', '')
        }
        const ro = new ResizeObserver(measure)
        ro.observe(nav)
        document.fonts?.ready.then(measure)
        return () => ro.disconnect()
    }, [])

    return null
}
