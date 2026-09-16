// Names and rules mirror crates/robin_engine/src/achievement.rs.
export const achievementRules: Readonly<Record<string, { readonly name: string; readonly description: string }>> = {
    "clean-hands": {
        "name": "Clean Hands",
        "description": "Complete the mission without player-caused deaths (NPC deaths also count when enabled)."
    },
    "ghost": {
        "name": "Ghost",
        "description": "Complete the mission without a living hostile observing a player character."
    },
    "pile-o-bones": {
        "name": "Pile-o-Bones",
        "description": "Place ten unconscious or dead NPCs in one building, then complete the mission."
    },
    "ruthless": {
        "name": "Ruthless",
        "description": "Complete the mission with every enemy dead."
    },
    "im-off-home": {
        "name": "I'm off home",
        "description": "Knock every rich civilian unconscious at least once, then complete the mission. They may wake up."
    },
    "charity": {
        "name": "Nothing in Return",
        "description": "Give a beggar money after all their information is exhausted, receiving no information in return."
    },
    "all-beggar-info": {
        "name": "Word on the Street",
        "description": "Get all information from every beggar and complete the mission. Mission badge only."
    },
    "no-banners-purchased": {
        "name": "Earned, Not Bought",
        "description": "Complete a banner mission without purchasing any of its preparation banners."
    },
    "all-banners-purchased": {
        "name": "Spare No Expense",
        "description": "Complete a banner mission having purchased every preparation banner, rather than earning any."
    },
    "a-legend-is-born": {
        "name": "A Legend Is Born",
        "description": "Complete the full campaign."
    },
    "for-king-richard": {
        "name": "For King Richard",
        "description": "Complete Lackland's Plan and send the ransom with Allan-a-Dale."
    },
    "whole-merry-company": {
        "name": "The Whole Merry Company",
        "description": "Complete the campaign after winning a mission with each of Robin's five named companions."
    },
    "no-empty-places": {
        "name": "No Empty Places at the Table",
        "description": "Complete the campaign without permanently losing a recruited hero or Merry Man, including strategic assignments."
    },
    "many-hands": {
        "name": "Many Hands Make Sherwood",
        "description": "Complete the campaign after three distinct generic Merry Men each contribute to a mission victory and complete production or training in Sherwood."
    },
    "kill-a-civilian": {
        "name": "Kill a Civilian",
        "description": "Kill a civilian during a successful mission, including indirect deaths. Civilians already dead at mission start do not count."
    },
    "leave-everyone-standing": {
        "name": "Leave Everyone Standing",
        "description": "Complete the mission unseen without harming or incapacitating any NPC. Distractions are allowed."
    },
    "not-a-scratch": {
        "name": "Not a Scratch",
        "description": "Complete the mission without any party member losing health."
    },
    "on-my-mark": {
        "name": "On My Mark",
        "description": "Have three characters successfully act on three distinct enemies in one quick-action execution, then win."
    },
    "you-never-saw-us-leave": {
        "name": "You Never Saw Us Leave",
        "description": "Escape three simultaneous pursuers without killing them or leaving the map, then win."
    },
    "string-theory": {
        "name": "String Theory",
        "description": "Complete a mission with three player-knocked-out enemies alive, bound, and inside a building."
    },
    "round-on-the-friar": {
        "name": "A Round on the Friar",
        "description": "Have three different enemies drink beer placed by Tuck in one successful mission."
    },
    "something-in-the-air": {
        "name": "Something in the Air",
        "description": "A single player-thrown wasp nest must sting three different enemies in a successful mission."
    },
    "different-kind-of-scarlet": {
        "name": "A Different Kind of Scarlet",
        "description": "Have Will Scarlet knock out six different enemies with his sling and finish with Clean Hands."
    },
    "people-behind-the-legend": {
        "name": "The People Behind the Legend",
        "description": "Win an optional ambush or tactical mission with only generic Merry Men."
    }
};
