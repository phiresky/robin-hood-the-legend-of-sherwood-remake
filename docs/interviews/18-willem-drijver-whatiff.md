# Willem Drijver on porting Robin Hood to Apollo Vampire V4

- Publication date: April 2024
- Publication: WhatIFF? Issue 3.13
- Original language: English

> Original-language transcript. The interview text is preserved as published; no translation has been applied. Follow the preserved-source links for the original files.

## WhatIFF? Issue 3.13 — Interview: Willem Drijver: Robin Hood — Robin Hood Apollo/Vampire Amiga port interview in WhatIFF? Issue 3.13

- Interview subject: Willem Drijver
- Original URL: https://whatiff.info/ag4web/db_ag2pdf.php?ag=WhatIFF_3.13_2024_Apr%2FWhatIFF-Issue-3.13.guide
- Wayback capture: not resolved
- Preserved source: [`14_willem-drijver-whatiff-3.13.pdf`](sources/14_willem-drijver-whatiff-3.13.pdf)
- Language: preserved as published; no translation applied

Interview - Willem Drijver: Robin Hood by Timo Paul

1. Hi Willem, thank you for agreeing to this interview. Can you please introduce yourself and tell us a bit about yourself?

Thank you for bringing us WhatIFF? magazine and I am more than happy to contribute with this interview. My full name is Willem Drijver and you can find me on Discord under my secret nickname @willemdrijver. I was born in the great year 1968 in the Netherlands, where I still live with my wife Monique in a small village near Utrecht.

My professional career started in Utrecht where I was studying "Informatica" (Computer Sciences) at the University around 1986-1990. At that time computers were bulky mainframes and acquiring access for coding time was a challenge. Together with an older friend, I started a business importing the Atari ST into the Netherlands, selling them to my fellow students and soon after also to my professors when they discovered the magic of a personal 68K computer for a small budget (starting from 2500 guilders = 3000 US$). From there the business grew steadily, first with a combined Atari/Amiga retail shop in downtown Utrecht, which gradually transformed into an IT-Services business and around 1999 the business moved to a large office and grew to 80 employees in 2017 at when I decided after long considerations to sell and retire.

2. How did you start using the Amiga?

Like many in Amiga community, my first real computer was a Commodore 64, where I spent countless hours playing (and pirating) Games and learning Basic programming. At the time I started my study I started using Atari ST (practice what you preach J), but my heart was stolen when the Amiga 1000 was introduced and from there I was lost forever. I do still have my first A1000 right here beside me, revived with an Apollo Firebird. My primary use for Amiga 1000, besides playing games of course, was coding, hacking and doing some amateur-level multi-media stuff.

3. Have you programmed games for the Amiga in the past?

I honestly have to admit that I am a lousy coder and although I did pass all the exams on Assembler, Pascal, C, etc I could never reach the level of some of my coding friends around me who were very productive in cracking games and/or creating their own demos. I was more intrigued with the commercial possibilities, although I always stayed in touch with my engineering roots (I actually was one of the first batch of Microsoft Engineers (MCSE) here in Holland.

4. You're currently working on converting Robin Hood: The Legend of Sherwood for the Vampire V4+ series of accelerators/standalone. Is this your first game for the Vampire platform?

Yes. The project of porting "Robin Hood" to Apollo V4 Series started from my personal wish to learn more about how real professional games are coded. When I restarted learning to code for Amiga on Apollo Vampire V2 and later V4 series, I admired even more than before the beauty of the Amiga architecture. The evolution by the brilliant mind of master Gunnar of that architecture to our current Apollo 68080 CPU and SAGA chipset is simply amazing, so when I was presented the opportunity to explore the fantastic game "Robin Hood: Legend of Sherwood" by Spellbound Studios and bring it to life on Apollo V4 I grabbed this with both hands.

5. What made you choose Robin Hood: The Legend of Sherwood?

This was a case of the right time and moment. On the one hand, I was looking into the coding structures of (Amiga) Games and on the other hand I stumbled by coincidence on Robin and his merry men. I did not know the game myself at all, so I watched some video clips and I immediately was captured by the beautifully detailed graphics and the whole immersive feeling of the game. After some initial studying, I decided to really make an effort to not only port the game but to also adapt it to Amiga style and of course optimize it using the powerful Apollo V4 features.

6. Why did you choose to develop for the Vampire platform instead of classic Amiga, OS4, or Morph OS?

Well, the Apollo Vampire V2 was the reason I rediscovered Amiga as it brought my A1000 back to life and I could re-live all my memories playing games. From there I restarted to take an interest in coding with ASM for the m68k and C for AmigaOS. The more I got into coding the more I understood of the great work that had been done developing the Apollo 68080 AMMX CPU and the SAGA special chipset. When the V4 arrived I had just joined the Team and was responsible for ApolloBoot as my personal project and later also coordinating ApolloOS development within the team. With all the additional features the new V4 Series offered I really started to dive into writing Apollo V4 optimized code. For me personally there is no feeling with either OS4 or MorphOS (this is not a qualification!).

7. What was the most challenging aspect of porting Robin Hood to the Vampire?

Robin Hood: Legend of Sherwood was released in two editions, the first was targeted for Windows and required 200Mhz+ Intel CPU to run. Development continued and in 2005 there was a release for OSX, which was again later in 2012 ported to Linux. For this version the requirements were lifted to 1Ghz+ CPU and a dedicated sound and graphics card. So, the main challenge (still today) is to get good and stable performance to get a smooth quality user experience the game deserves.

To overcome this challenge, I need to understand the complete inner workings and identify all the critical data structures, routines and dependencies. Step by step I have recoded all these critical parts, using Apollo specific features both from C++ level as well as in a lot of dedicated developed Apollo ASM routines.

Time will tell if I (with the help of my teammates) will succeed in getting Robin Hood to a quality level that is sufficient for any form of release. Remember that the whole project is started as an internal and personal experiment, and I never expected to get as far as I have at this point.

8. Will the port run on the V2 or only the V4 series?

Given the challenge described in the previous question, I would already be very happy to get the game running smooth on V4. I very much doubt if it will be possible to make this happen for V2, but I also do not rule out anything at this point.

9. How has the advanced Vampire hardware made the game easier to port?

Apollo 68080 architecture is clean and simple and yet very powerful for developers using familiar 68k style registers and instructions. For example I could relatively easy rewrite the complete Video and Audio parts of Robin which were based on the fine but bulky SDL framework. Dozens of complex Audio routines for handling starting/stopping/mixing channels were replaced by new code with only a few lines and all critical Graphic routines were replaced by short ASM routines which typically take no more than 25 instructions or so. Within my scope of skills and experience this could only be done on the Apollo V4 platform.

10. What is the current stage of the game's development?

It’s fair to say that we are beyond our "experimental" level, and I believe a "beta" stage would be the appropriate qualification. There is still some work to do in squeezing out extra FPS to really make Robin and all his friend come fully to live on the V4’s of our growing and loyal Apollo community. Also, the game profile manager which handles save games and such is still a pain in the lower part of my body (Never Endian Story). Finally, all our development testing is done in the one "demo level" and I kind of expect some bumps in the road when we start travelling through a grand total of no less than 16 complex Missions.

11. When do you expect the game to be released?

It’s still too early to give any indication for that. Amiga community is full of promises, but sadly a lot of them are no more than that. I would like to take the time needed to finish something properly.

12. What game would you like to create or port next for the Amiga platform?

After Robin I will likely take a pause from Game coding/porting and work more on ApolloBoot and ApolloOS. But in the longer run it would be logical to look at Desperados as this is the "twin" from Robin. Also, I have an idea to develop a simple but fun Game fully coded in ASM. I already have the graphics for this, designed by Apollo Team friend Kevin Saunders.

13. For Amiga users interested in buying a Vampire card or StandAlone system, what advice do you have for them?

Don`t hesitate and just make the step. Apollo V4 Stand Alone as well as the V4 IceDrake (A1200), V4 FireBird (A500/1000/2000) and V4 MantiCore (A600) accelerators will bring you into a revived Amiga world where you can enjoy both the retro history as well as exciting new developments which come from our very active Team and Community. Visit our Discord server and become part of our little crazy, but always positive group of Amiga fans.

14. Thank you for your time. Do you have any final words before we wrap up?

Again, it is me thanking you and your colleagues for spending precious time on creating WhatIFF magazine and keeping us all informed on all the interesting Amiga stuff. I truly believe that the Amiga community can become more positive and cohesive by means of better communication and information. This is why initiatives like WhatIFF magazine are important. Regarding questions or chats on Robin Hood, ApolloBoot, ApolloOS and other things I am to be found on Apollo Discord (@willemdrijver). Thank you all for reading and have fun with Amiga in whatever shape or form is most suitable for you.

Cheers!
