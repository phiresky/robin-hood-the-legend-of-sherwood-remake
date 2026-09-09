# graphics dilemma with "Robin Hood"

- Source: [PC Review forum thread](https://www.pcreview.co.uk/threads/graphics-dilemma-with-robin-hood.514210/), thread 514210
- Forum: Windows XP Games
- Thread starter: Guest (signing the post as Manuel)
- Date: July 29, 2004
- Language: English
- Retrieved: September 9, 2026

## Original thread

### Guest (signing as Manuel), July 29, 2004

I'm experiencing a problem with a certain game I installed on my XP. Every time I play "Robin Hood- The legend of Sherwood", approx. 10 minutes through play the screen/display continuosly blinks and the mouse cursor leaves a trail of boxes compiled of the background image. I don't understand why I would have trouble with the game when my directX is functioning OK. If someone would be able to point me in the right direction, I'd be much obliged.

### Jimmy S., July 29, 2004

> I'm experiencing a problem with a certain game I installed on my XP. Every time I play "Robin Hood- The legend of Sherwood", approx. 10 minutes through play the screen/display continuosly blinks and the mouse cursor leaves a trail of boxes compiled of the background image. I don't understand why I would have trouble with the game when my directX is functioning OK. If someone would be able to point me in the right direction, I'd be much obliged.
>
> — Manuel

Hi Manuel,

Updating video card drivers can solve most gaming issues. Here's some simple abc's to always keep in mind.

a. Shut off download accelerators, firewalls and antivirus programs when downloading or installing updates;

b. Check for game patches: [www.avault.com/pcrl/patches_list.asp?letter=a](http://www.avault.com/pcrl/patches_list.asp?letter=a)

c. Make sure you meet the game's minimum video and system requirements.

Before you update your drivers, I recommend that you install DirectX 9.0b: <http://download.microsoft.com/download/c/9/c/c9c8a1d4-7690-4c98-baf3-0c67e7f3751f/dx90update_redist.exe>

Here are the steps I recommend you use to update your driver:

1. To identify the make and model of your card, right click your Desktop, choose Properties / Settings / Advanced / Adapter.

2. Download the latest video driver for your card online, using <http://www3.sympatico.ca/nibblesnbits/Video.html#drivers> to find the website to download from. I also have advanced video driver and direct X troubleshooting steps on that page.

3. Save the .exe driver (or extract the zip file) to a folder in My Documents named after the driver version number.

4. Restart the computer in Safe Mode by pressing the F8 key about once every second as it's rebooting to pick Safe Mode.

5. Click Start / (settings) Control Panel / System / Hardware Device Manager / expand +Display Adapters / right click on the adapter, pick "Uninstall", and click No if asked to reboot.

6. Use Control Panel / Add-Remove programs to uninstall the previous driver (exe)software which may have been installed.

7. Restart the computer in Safe Mode by pressing the F8 key about once every second as it's rebooting to pick Safe Mode.

8. If the driver is NOT a (.exe)program file, GO TO step 11.

9. When Windows prompts you to install the video adapter, click "Cancel" and Double click the driver program to begin installation.

   * Even if not prompted, doubleclick the driver and install it.

10. After you reboot, go to Control Panel / Display / Settings and choose 32 bit Color Quality, and 800x600 or higher Resolution. That's it! Scroll down to the Troubleshooter if you have problems.

11. When Windows prompts you to install the video adapter, click "Install from a list or specific location", click the "Browse" button, browse to the My documents folder where you saved the driver, and finally click on one of the driver files to begin installation.

   ** If you are not prompted, or if the driver was updated automatically:** Click Start / (settings) Control Panel / System / Hardware Device Manager expand +Display Adapters / right click on the adapter, pick "Update Driver" to start the Update Wizard, choose the "Install from a list..." option: Browse to the My Documents\ folder with the driver in it. Click OK and click Next to begin the update.

12. After you reboot, go to Control Panel / Display / Settings and choose 32 bit Color Quality, and 800x600 or higher Resolution. That's it! Try the advice in the Troubleshooter if you have problems.

**TROUBLESHOOTING:**

Test your drivers using DXDiag: Click Start / Run / type: DXDIAG. Click the "Test" buttons in the Display, Sound, Music & Network Tabs; If any of the Display options are Disabled and you cannot Enable them, your most likely solution would be to update your Chipset Drivers as per my website: <http://www3.sympatico.ca/nibblesnbits/Video.html#v11>

Your program might not support dual monitors, or "dual head" video cards. You can disable the extra video output in your display properties control panel. Click Start>Settings>Control Panel>Display>Settings>Advanced.

Along with your Video card, Sound Cards, Motherboard Chipsets, and Video Monitors may also require updated drivers. Even your motherboard's BIOS may need to be updated for compatibility with your Video card. These steps are listed at: <http://NibblesNbitsVideo.tk>

Perhaps the old Video drivers did not completely uninstall. If that's the case, use these utility to completely uninstall the drivers and go to step 7:

- nvidia: [http://content.guru3d.com/index.php?page=detonatorrip&menu=0](http://content.guru3d.com/index.php?page=detonatorrip&menu=0)
- or for all cards including nvidia use: <http://www.driverheaven.net/cleaner/>

The latest video drivers sometime don't work with a particular game. (Check the Video suggestions in the readme.txt file in your game folder/CD) If there's no suggestions, try an older (WHQL) driver, and/or if you still experience problems try a Beta driver, or even an Omega driver instead:

**BETA Drivers:** <http://download.guru3d.com/>

**OMEGA Drivers:** <http://www.omegacorner.com/>

There you have it, if you have any questions feel free to post them!

--

Cheers, Windows XP MVP Shell / User
Jimmy S. <http://mvp.support.microsoft.com>

Game FAQs: <http://support.microsoft.com/default.aspx?scid=FH;[LN];gms>
Visit my Zone.com / Gaming Helpsite: <http://nibblesnbits.tk> or Call / Contact MS Support at: <http://support.microsoft.com/default.aspx?scid=sz;en-us;top>

My advice is donated "AS IS" without warranty; nor do I confer any rights.

---

Editorial note: The archived thread contains these two posts and no follow-up reporting an outcome. It gives no hardware, driver-version, game-version, or diagnostic-log details, so the symptom is evidence of an early Windows XP report but does not establish the same cause as later wrapper-era cursor or flip-chain issues.
