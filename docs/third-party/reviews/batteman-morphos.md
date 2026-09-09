# BatteCave — On m'appelle Robin des Bois !

- Original source: [BatteCave — On m'appelle Robin des Bois !](https://www.batteman.com/2012/05/test-on-mappelle-robin-des-bois.html)
- Author / publication: BatteMan / BatteCave
- Language / date: French; posted 2012-05-03, originally written 2007-02-08
- Access: Article text inspected
- Checked: 2026-09-09
- Format: original summary and research notes; not a transcript.

This firsthand MorphOS account enjoys the tactical play, camp production, and training, but documents substantial instability. The author reports roughly ten crashes over nine hours, alongside occasional sessions lasting several hours without trouble.

A mission or save-loading failure at 78% prevented this particular playthrough from finishing. Alternate executables supplied by the porter did not resolve it, although other players reportedly completed the game. The article also mentions slowdown at 800×600 on a Pegasos II G4.

PowerSDL is offered as an explanation for crashes in the account, not established by independent debugging here. These observations belong to that historical port and machine, rather than every edition.

## Converted text from the original HTML


### reviews__batteman-morphos.html

_Source: `originals/reviews__batteman-morphos.html`._

[ ](https://www.batteman.com/)

La BatteCave, c'est : un peu d'Amiga, pas mal de jeux vidéo (sur consoles mais aussi Amiga) et un tout petit peu de cinéma.

## Pages

  * [Accueil](https://www.batteman.com/)
  * [Sommaire](https://www.batteman.com/p/sommaire.html)
  * [Blogs amis](https://www.batteman.com/p/blogs-amis.html)



## jeudi 3 mai 2012

###  [TEST] On m'appelle Robin des Bois ! 

[ ](https://draft.blogger.com/post-edit.g?blogID=6650103059912426987&postID=8096709103569603686&from=pencil "Modifier l'article")

[](https://blogger.googleusercontent.com/img/b/R29vZ2xl/AVvXsEhPMmVn2W5EQLR3XjnZjvArx_43GiTF4xsyKkCK-Mhs1cXiOlj0DKBej7_3V8DPe6obO5tziLNXBQZmOJokoKLeJidSrTBAfe8cLgj3RVSjp3QoirNjsqgexKElqgO6skJ1OPYE5sV-dxY/s1600/banniere.jpg)

 **Quand Robin des bois vole le PC pour donner à MorphOS !**

  


**RuneSoft** , ex **Epic Software** , avait annoncé travailler sur un portage de "**Robin Hood: The Legend of Sherwood** " (jeu de **Spellbound Entertainment AG** , sorti sur PC en 2002 et publié par... **Wanadoo Editions** , wouhou le coup de vieux ^^) pour MorphOS en décembre 2004. Depuis lors, peu de choses avaient filtré et on se demandait si Robin n'allait pas rejoindre "**Divine Divinity** " et "**Northland** " au paradis des hommes sans tête (les cinéphiles auront reconnus une citation du très bon **Dobermann** de Jan Kounen). Après un portage vers Linux PPC rendu disponible en juillet 2006, tout semblait alors de nouveau possible. En octobre 2006, la version MorphOS était finalement annoncée, accompagnée d'une démo jouable. Fin novembre, le jeu est enfin disponible à la vente et, une bonne nouvelle ne venant jamais seule, incluant même la version française. En plus de la sortie de "Robin Hood", RuneSoft a également annoncé que les portages de "**Airline Tycoon Deluxe** " et de "**Chicago 1930** " étaient en cours, et que celui de "Northland" avait repris de zéro. Mais voilà, mis à part Robin et "Airline Tycoon", nous ne verrons pas l'ombre d'un pixel des deux derniers, qui étaient pourtant les plus intéressants, à mon humble avis... 

  
  


**  Premiers contacts avec Robin**

  


Tout a commencé quand le portage a été annoncé. À l'époque, j'avais testé la démo sous MacOSX (via Mac-On-Linux qui permettait d'émuler, de manière très honorable, un Mac PPC sur mon Pegasos II) et j'avais été agréablement surpris en voyant le jeu tourner aussi bien. Connaissant déjà le concept de ce genre de jeu (j'avais joué à "**Commandos** " PC chez un ami et j'avais bien avancé dans le jeu), je trépignais donc d'impatience de voir la version MorphOS. Il aura fallu attendre près de deux ans mais Robin est arrivé et j'ai pu me casser les dents sur la démo. Et là, ce fût le drame... En effet, la [](https://blogger.googleusercontent.com/img/b/R29vZ2xl/AVvXsEjBDSiXyH6HM25Uro58bYAKjjU8TWJCcxe5_W4_2i57mrTNc1JlzOxqAhWrY4uUb-zxZeQucUJvTI_BNV8nIHGIxylmbIhoafByYGAy42JTb4kPM9qUrgYIqoxT2jV35FVtDPyDYBKe89w/s1600/Chargement.png)première démo ne tournait correctement qu'en 640*480 sur un Pegasos II G4, je n'osais alors imaginer ce qu'elle donnait sur un Pegasos I G3. Peu de temps après, une nouvelle version de la bibliothèque PowerSDL est sortie et a corrigé ce souci de lenteur, rendant le jeu presque parfaitement jouable en 1024*768. Après m'être amusé sur la démo, j'ai commandé le CD chez feu **FLComputer** , une boutique de VPC située au Luxembourg et qui vendait du matériel et des logiciels Amiga. Ce dernier, tout comme **RELEC** ou encore **Amikit** et **GGS-Data** à l'époque, possédait le CD comportant la version française et anglaise. Si vous le commandiez ailleurs, vous aviez la version anglaise et allemande, il fallait donc faire attention au moment de passer la commande. Passons maintenant à l'installation du jeu. Eh oui, il va falloir l'installer entièrement sur le disque dur, soit près de 950 Mo. Le miracle de la compression tar.bz permet donc de faire tenir plus de 1,2 Go sur un CD, pas mal. L'installation utilise un simple script qui va désarchiver l'archive commune et l'archive de la langue dans le répertoire choisi. Dix minutes plus tard, vous n'avez plus qu'à cliquer sur l'icone de Robin et à jouer. 

  


La première chose qui frappe, c'est la qualité de la cinématique d'introduction, très jolie, rythmée et longue. Elle a, de plus, l'intelligence de nous plonger directement dans le bain et de nous préparer à vivre les aventures de sire Robin de Locksley, dépouillé de ses biens par le shériff de Nottingham et par le Prince Jean Sans Terre qui gouverne le royaume en l'absence de son frère, le roi Richard Coeur de Lion. Tout le monde connaît l'histoire, mais c'est avec un réel plaisir qu'on se prend à incarner Robin et ses compagnons de la forêt de Sherwood. 

  
  
**Une histoire connue**  
  


Évidemment, le scénario de Robin Hood est connu d'avance et les grandes étapes de l'histoire se déroulent sans surprises. On commence en revenant des croisades, et l'on [](https://blogger.googleusercontent.com/img/b/R29vZ2xl/AVvXsEgM20vhnFnHgqQZQu4GL7xxf4Vl2GyrLZPe7yO7OslzU2SV89uxJVz_3Iu5PErIjbmXovugSlUL0JAy_g-iuorvjE9gcqz5xznlyhcoHn55ENR-kTl-u6ntJPgu0AlEf5M4nfCne_9t5Ik/s1600/DeLoin.png)se rend compte qu'on a été déclaré mort et que nos terres ont été confisquées... Après avoir délivré quatres amis de la potence, on se rend finalement dans la forêt de Sherwood, le point névralgique de toute l'histoire. En effet, c'est ici que vos compagnons produiront des flèches, des herbes de soins, de la viande, s'entraîneront au combat ou au tir à l'arc, et fabriqueront toutes sortes d'objets plus ou moins utiles suivant votre façon de jouer. À chaque retour de mission, vous pourrez ainsi ravitailler vos hommes afin qu'ils soient prêts à repartir dans de nouvelles aventures. Après cela, vous irez délivrer votre cousin Will Scarlet, vous rencontrerez Dame Marianne, délivrerez Frère Tuck, participerez au concours de tir à l'arc afin de remporter la flèche d'argent et le baiser de la douce Marianne, etc. Au cours de votre aventure, vous vous rendrez également compte que le Prince Jean n'est pas digne de gouverner le royaume. En effet, il ne veut même pas débourser 100.000 livres pour la rançon du Roi, son frère, et spécule sur sa mort pour s'emparer du trône. Robin devra donc voler encore et toujours afin d'amasser le butin nécessaire pour délivrer le seul véritable détenteur de la couronne d'Angleterre.

  
  
**Le jeu en lui même**  
  


Il s'agit d'un jeu d'infiltration/action où vous devrez remplir des missions prédéfinies (des missions annexes et non obligatoires apparaissent également en cours de mission). Les missions sont variées, comme on a pu le voir précedemment, mis à part les missions intermédiaires où vous devrez uniquement amasser assez d'argent pour continuer. Et là, si vous avez eu la mauvaise idée de dépenser votre argent inutilement, vous serez dans l'obligation d'enchaîner une dizaine de missions quasi identiques sur l'une des quatre cartes dévolues au racket et à la rapine. Car oui, ces missions ne se déroulent que sur quatre cartes et je peux vous assurer que l'on a vite fait de les connaître... Niveau jouabilité, c'est tout bonnement simple et efficace. Vos hommes répondent à vos [](https://blogger.googleusercontent.com/img/b/R29vZ2xl/AVvXsEjl1MNC5vjA4bDiCNdZKtqmX5xB4Yzd79xSQkrsXnSmhBKgCUk_tlZQecpi5xU0c-jB3QUD6zGYq9M2se8iXqCfZsFDNvmI0Iu7VqTmAmcilQC7_PQxVmgfuPWKSbSukbnJXEPMZUTfJnE/s1600/DeNuit.png)ordres sans broncher et l'on se prend à réfléchir au meilleur chemin pour passer sans se faire repérer, comment attirer les ennemis un par un pour les assommer tranquillement. De plus, chaque personnage a des actions particulières. En effet, seul Robin et Petit Jean peuvent assommer des gardes, seul Tuck peut lancer des nids d'abeilles sur les gardes, seul Will Scarlet peut jouer de la fronde, Marianne est la seule à avoir l'ouïe assez fine pour savoir si ce sont des gardes ou de simples paysans qui sont cachés derrière un mur, etc. Vous devez donc composer votre équipe avec soin avant de commencer vos missions sous peine de ne pas réussir, ou pire, de tuer tout vos compagnons. Autre détail sympathique, vous gérez vous-même les combats. Quand un des personnages que vous avez sélectionné se bat, vous indiquez les mouvements à faire à l'aide de la souris : un cercle à droite et votre personnage fait un coup rotatif à droite, idem à gauche, le coup le plus mortel mais aussi le plus lent est le huit couché. Si vous jouez avec Petit Jean, utilisez les cercles. Cela vous permettra d'assommer tout ceux qui se trouvent autour de lui, mais attention, vos compagnons eux aussi sont susceptibles d'être touchés par les coups du grand Petit Jean. Les missions s'achèvent une fois l'objectif principal effectué, mais vous pouvez choisir de rester dans le niveau afin de ramasser des trésors cachés ici où là, ou bien pour finir les missions annexes que vous n'avez pas encore eu le temps de faire. Ceci permet de laisser une liberté d'action et de choix au joueur, ce qui n'est pas négligeable. De plus, les niveaux et les missions sont faits de manière à laisser plusieurs solutions : soit vous foncez dans le tas, soit vous passez discrètement par les toits, ou bien vous essayez de vous infiltrer en vous cachant dans les maisons aux portes ouvertes, etc. Ceci vous permettra de refaire les missions plus tard sans pour autant avoir l'impression d'y avoir déjà joué.

  


Les décors et les personnages sont en 2D mais les graphismes sont si fins et l'angle de caméra tellement [](https://blogger.googleusercontent.com/img/b/R29vZ2xl/AVvXsEhDk7PcBnoTih_v5wMOwljn3uoZOoKbP7-nABlr2AaAyo-wg3G0uI6vyCNdLhi74W3IDaT1mFz6lbhxwCcrZqmM9yEQYLHTVHxRd5nQcmF0vuCwAbhN-joDNbF40RkaiEEMWrxTZ1s6rMw/s1600/Ville.png)judicieux qu'on se croirait dans des décors en 3D. Vous avez trois niveaux de zoom qui permettent de se rapprocher de l'action ou de s'en éloigner. De plus, les développeurs ont eu la bonne idée de nous permettre de rentrer dans les bâtiments. Pour pouvoir continuer de jouer tout en étant à l'intérieur, les bâtiments seront alors affichés "en écorché" et vous pourrez profiter des intérieurs des châteaux, encore un vrai plaisir pour la rétine. L'ambiance sonore n'a pas été délaissée non plus puisque les musiques sont discrètes mais collent bien à l'ambiance. Les bruitages donnent de la vie aux cartes et vos personnages iront de leurs petites piques ou remarques. Des séquences de dialogues entièrement doublées en français sont aussi de la partie. De ce côté là, rien à redire.

  
  
**Le mais qui fait mal...**  
  


Eh oui, vous vous dites que ce jeu est un petit bijou et que vous allez essayer de trouver un exemplaire de ce jeu pour y jouer dès que possible. Si vous voulez y jouer [](https://blogger.googleusercontent.com/img/b/R29vZ2xl/AVvXsEh4Mb7mInNp3huFVM_Vwl-qyFchY6KmL88m0JW_kNjjUFXVrXLZesgPe4ZSehpevORvwBXdOggx9U-MA09md3dwgOrUy0IgeO_l9EWOlLPLXdKiJElgSdNR8RCjV49VkvHyHP-kJupJRmk/s1600/Zoom.png)sur PC ou Mac, foncez, il ne devrait pas y avoir de souci. Par contre, si c'est pour y jouer sur une machine MorphOS, attendez avant de vous jeter car le jeu dans cette version souffre d'un gros soucis... Il a une tendance au suicide qui peut en énerver plus d'un. Bien sûr, en sachant cela, on fait des sauvegardes fréquentes et on arrive à avancer sans trop d'embûches dans le jeu. La preuve, j'ai fait 78% du jeu sans trop me plaindre de ces plantages (qui doivent quand même être de l'ordre de la dizaine en 9 heures de jeu...). Cependant, il m'est arrivé de jouer trois à quatre heures de suite sans aucun souci. Mais il faut reconnaître que c'est génant, énervant voire frustrant (surtout quand on oublie de sauvegarder) mais cela reste encore facilement surmontable. Et puis cela vous évitera de rester trop longtemps devant votre écran ^^   
Mais, car il y a un autre mais, j'ai rencontré un autre soucis. Apparement, je suis le [](https://blogger.googleusercontent.com/img/b/R29vZ2xl/AVvXsEjZK5bp-ck-YXi9ORt2t-maB_myPyF8kgcrRFz-lDVH2bY5z9WbuApxs28dHXPcyo9V20CSSvjJnFHtJ5AMdEqj-rXurx4W04hnziBhMe_NItUIqH-nCXmyPYERVnSSGt0Ca1Q_ISvvI-4/s1600/BatimentEcorche.png)seul à m'en plaindre mais tout de même. En effet, le jeu plante au chargement de la mission se passant à 78% du jeu et m'empêche tout bonnement de finir le jeu. Quelques personnes que je connais et qui ont le jeu n'ont rencontré aucun soucis et ont réussi à le terminer. C'est vraiment rageant. Je suis entré en contact avec le développeur qui a porté Robin sur MorphOS et ce dernier m'a bien donné d'autres exécutables sans plus de succès. Concernant les plantages à répétition, ils ne viendraient pas du jeu en lui-même mais de la bibliothèque PowerSDL, ce qui laisse espérer une amélioration au fil du temps (je dois avouer que je n'ai pas retenter l'expérience depuis toutes ces années...). Quant à mon problème de sauvegarde, je crois que je vais devoir me résigner à recommencer depuis le début (ce qui n'est pas plus mal, puisque maintenant je sais comment jouer correctement) ou alors tenter de repartir d'une autre sauvegarde un peu moins récente afin de voir si je pourrais contourner ce problème. 

  
  
**"On m'appelle Robin des Bois Je m'en vais par les champs et les bois. Et je chante ma joie par dessus les toits."**   
  


**Le refrain de la chanson que chantait****Georges Guétary dans l'opérette "Robin des bois" de 1943 résume parfaitement ce "Robin Hood : La légende de Sherwood". Ce dernier est composé de décors variés (vous naviguez entre champs, bois, châteaux, villes) et est le jeu parfait pour tout ceux qui sont fans des jeux d'infiltration/action. Si vous connaissez "Commandos"(version PC ou PS2, pas le vieux "Commandos" Amiga ^^) et que vous aimez, alors vous ne serez pas déçu. Évidemment, il reste le souci des plantages, dans sa version MorphOS, qui émaille énormément ce qui pourrait être une perle... Mais ce soucis, bien que pénalisant, peut être contourné par des sauvegardes fréquentes. Le plus gros soucis vient sans doute de mon problème de sauvegarde qui ne touche que moi apparemment... Alors que dire de Robin ? Très bon jeu, fort sympathique et qui est d'ailleurs le seul représentant du genre sur Amiga/MorphOS. Agréable à jouer, simple, voire trop simple pour certains, beau mais ayant encore quelques soucis de vitesse en 800*600 lors de certains passages sur un Pegasos II G4. Finalement, la note que Daff lui a donné sur Obligement, à savoir 8,5/10, me semble bonne si on ne tient pas compte des soucis de stabilité. Quel dommage... Espérons toutefois qu'une mise à jour de la PowerSDL arrivera un jour à corriger ces défauts qui pourraient s'avérer rédhibitoire pour certains.**

  
 \--  
/me avait "surkiffé" ce Robin à l'époque ! ^^  
  


_Billet posté le 3 mai 2012 (initialement écrit le 8 février 2007)_

Posted by  [ BatteMan ](https://draft.blogger.com/profile/12815652402163334962 "author profile") Labels: [[AMIGA]](https://www.batteman.com/search/label/%5BAMIGA%5D), [[TEST]](https://www.batteman.com/search/label/%5BTEST%5D), [Amiga](https://www.batteman.com/search/label/Amiga), [Epic Software](https://www.batteman.com/search/label/Epic%20Software), [Mac](https://www.batteman.com/search/label/Mac), [MorphOS](https://www.batteman.com/search/label/MorphOS), [PC](https://www.batteman.com/search/label/PC), [Robin Hood: The Legend of Sherwood](https://www.batteman.com/search/label/Robin%20Hood%3A%20The%20Legend%20of%20Sherwood), [RuneSoft](https://www.batteman.com/search/label/RuneSoft), [Spellbound Entertainment AG](https://www.batteman.com/search/label/Spellbound%20Entertainment%20AG)

[Envoyer par e-mail](https://draft.blogger.com/share-post.g?blogID=6650103059912426987&postID=8096709103569603686&target=email "Envoyer par e-mail")[BlogThis!](https://draft.blogger.com/share-post.g?blogID=6650103059912426987&postID=8096709103569603686&target=blog "BlogThis!")[Partager sur X](https://draft.blogger.com/share-post.g?blogID=6650103059912426987&postID=8096709103569603686&target=twitter "Partager sur X")[Partager sur Facebook](https://draft.blogger.com/share-post.g?blogID=6650103059912426987&postID=8096709103569603686&target=facebook "Partager sur Facebook")[Partager sur Pinterest](https://draft.blogger.com/share-post.g?blogID=6650103059912426987&postID=8096709103569603686&target=pinterest "Partager sur Pinterest")

#### Aucun commentaire:

#### Enregistrer un commentaire

[](https://draft.blogger.com/comment/frame/6650103059912426987?po=8096709103569603686&hl=fr&saa=85391&origin=https://www.batteman.com)

[Article plus récent](https://www.batteman.com/2012/05/cinema-une-histoire-de-violence.html "Article plus récent") [Article plus ancien](https://www.batteman.com/2012/04/livre-allez-gunpei-prends-un-yoco-ca.html "Article plus ancien") [Accueil](https://www.batteman.com/)

Inscription à : [Publier les commentaires (Atom)](https://www.batteman.com/feeds/8096709103569603686/comments/default)

## BattOnline

[](https://batteman.itch.io/) [](https://mastodon.world/@batteman) [](https://www.instagram.com/batteman/) [](http://www.batteman.com/feeds/posts/default)

## BatteSearch

|   
---|---  
  
## BatteBillets

Your browser does not support JavaScript!

## BatteTweets

## BattAgr.am

[Compte Instagram](https://www.instagram.com/batteman/)  

## BatteComm'

## BattArchives

  * [ ► ](javascript:void\(0\)) [ 2019 ](https://www.batteman.com/2019/) (1)
    * [ ► ](javascript:void\(0\)) [ août ](https://www.batteman.com/2019/08/) (1)


  * [ ► ](javascript:void\(0\)) [ 2018 ](https://www.batteman.com/2018/) (1)
    * [ ► ](javascript:void\(0\)) [ mai ](https://www.batteman.com/2018/05/) (1)


  * [ ► ](javascript:void\(0\)) [ 2017 ](https://www.batteman.com/2017/) (7)
    * [ ► ](javascript:void\(0\)) [ octobre ](https://www.batteman.com/2017/10/) (2)
    * [ ► ](javascript:void\(0\)) [ septembre ](https://www.batteman.com/2017/09/) (1)
    * [ ► ](javascript:void\(0\)) [ août ](https://www.batteman.com/2017/08/) (3)
    * [ ► ](javascript:void\(0\)) [ janvier ](https://www.batteman.com/2017/01/) (1)


  * [ ► ](javascript:void\(0\)) [ 2016 ](https://www.batteman.com/2016/) (1)
    * [ ► ](javascript:void\(0\)) [ octobre ](https://www.batteman.com/2016/10/) (1)


  * [ ► ](javascript:void\(0\)) [ 2015 ](https://www.batteman.com/2015/) (1)
    * [ ► ](javascript:void\(0\)) [ octobre ](https://www.batteman.com/2015/10/) (1)


  * [ ► ](javascript:void\(0\)) [ 2014 ](https://www.batteman.com/2014/) (4)
    * [ ► ](javascript:void\(0\)) [ novembre ](https://www.batteman.com/2014/11/) (1)
    * [ ► ](javascript:void\(0\)) [ octobre ](https://www.batteman.com/2014/10/) (3)


  * [ ► ](javascript:void\(0\)) [ 2013 ](https://www.batteman.com/2013/) (11)
    * [ ► ](javascript:void\(0\)) [ octobre ](https://www.batteman.com/2013/10/) (3)
    * [ ► ](javascript:void\(0\)) [ août ](https://www.batteman.com/2013/08/) (5)
    * [ ► ](javascript:void\(0\)) [ mars ](https://www.batteman.com/2013/03/) (1)
    * [ ► ](javascript:void\(0\)) [ février ](https://www.batteman.com/2013/02/) (1)
    * [ ► ](javascript:void\(0\)) [ janvier ](https://www.batteman.com/2013/01/) (1)


  * [ ▼ ](javascript:void\(0\)) [ 2012 ](https://www.batteman.com/2012/) (22)
    * [ ► ](javascript:void\(0\)) [ novembre ](https://www.batteman.com/2012/11/) (1)
    * [ ► ](javascript:void\(0\)) [ octobre ](https://www.batteman.com/2012/10/) (1)
    * [ ► ](javascript:void\(0\)) [ septembre ](https://www.batteman.com/2012/09/) (1)
    * [ ► ](javascript:void\(0\)) [ août ](https://www.batteman.com/2012/08/) (1)
    * [ ► ](javascript:void\(0\)) [ juin ](https://www.batteman.com/2012/06/) (1)
    * [ ▼ ](javascript:void\(0\)) [ mai ](https://www.batteman.com/2012/05/) (6)
      * [[TEST] L'adage des assassins](https://www.batteman.com/2012/05/test-ladage-des-assassins.html)
      * [[CINÉMA] Le dormeur doit se réveiller !](https://www.batteman.com/2012/05/cinema-le-dormeur-doit-se-reveiller.html)
      * [[CINÉMA] Ducky Duke](https://www.batteman.com/2012/05/cinema-ducky-duke.html)
      * [[CINÉMA] Le nom de baptème ?](https://www.batteman.com/2012/05/cinema-le-nom-de-bapteme.html)
      * [[CINÉMA] Une histoire de violence ?](https://www.batteman.com/2012/05/cinema-une-histoire-de-violence.html)
      * [[TEST] On m'appelle Robin des Bois !](https://www.batteman.com/2012/05/test-on-mappelle-robin-des-bois.html)
    * [ ► ](javascript:void\(0\)) [ avril ](https://www.batteman.com/2012/04/) (4)
    * [ ► ](javascript:void\(0\)) [ mars ](https://www.batteman.com/2012/03/) (1)
    * [ ► ](javascript:void\(0\)) [ février ](https://www.batteman.com/2012/02/) (5)
    * [ ► ](javascript:void\(0\)) [ janvier ](https://www.batteman.com/2012/01/) (1)


  * [ ► ](javascript:void\(0\)) [ 2011 ](https://www.batteman.com/2011/) (13)
    * [ ► ](javascript:void\(0\)) [ novembre ](https://www.batteman.com/2011/11/) (1)
    * [ ► ](javascript:void\(0\)) [ septembre ](https://www.batteman.com/2011/09/) (4)
    * [ ► ](javascript:void\(0\)) [ juin ](https://www.batteman.com/2011/06/) (3)
    * [ ► ](javascript:void\(0\)) [ mai ](https://www.batteman.com/2011/05/) (1)
    * [ ► ](javascript:void\(0\)) [ avril ](https://www.batteman.com/2011/04/) (1)
    * [ ► ](javascript:void\(0\)) [ février ](https://www.batteman.com/2011/02/) (1)
    * [ ► ](javascript:void\(0\)) [ janvier ](https://www.batteman.com/2011/01/) (2)


  * [ ► ](javascript:void\(0\)) [ 2010 ](https://www.batteman.com/2010/) (24)
    * [ ► ](javascript:void\(0\)) [ septembre ](https://www.batteman.com/2010/09/) (3)
    * [ ► ](javascript:void\(0\)) [ août ](https://www.batteman.com/2010/08/) (3)
    * [ ► ](javascript:void\(0\)) [ avril ](https://www.batteman.com/2010/04/) (6)
    * [ ► ](javascript:void\(0\)) [ mars ](https://www.batteman.com/2010/03/) (6)
    * [ ► ](javascript:void\(0\)) [ février ](https://www.batteman.com/2010/02/) (5)
    * [ ► ](javascript:void\(0\)) [ janvier ](https://www.batteman.com/2010/01/) (1)


  * [ ► ](javascript:void\(0\)) [ 2009 ](https://www.batteman.com/2009/) (40)
    * [ ► ](javascript:void\(0\)) [ décembre ](https://www.batteman.com/2009/12/) (1)
    * [ ► ](javascript:void\(0\)) [ octobre ](https://www.batteman.com/2009/10/) (1)
    * [ ► ](javascript:void\(0\)) [ septembre ](https://www.batteman.com/2009/09/) (1)
    * [ ► ](javascript:void\(0\)) [ août ](https://www.batteman.com/2009/08/) (7)
    * [ ► ](javascript:void\(0\)) [ juillet ](https://www.batteman.com/2009/07/) (2)
    * [ ► ](javascript:void\(0\)) [ juin ](https://www.batteman.com/2009/06/) (1)
    * [ ► ](javascript:void\(0\)) [ mai ](https://www.batteman.com/2009/05/) (2)
    * [ ► ](javascript:void\(0\)) [ avril ](https://www.batteman.com/2009/04/) (6)
    * [ ► ](javascript:void\(0\)) [ mars ](https://www.batteman.com/2009/03/) (15)
    * [ ► ](javascript:void\(0\)) [ février ](https://www.batteman.com/2009/02/) (3)
    * [ ► ](javascript:void\(0\)) [ janvier ](https://www.batteman.com/2009/01/) (1)


  * [ ► ](javascript:void\(0\)) [ 2008 ](https://www.batteman.com/2008/) (19)
    * [ ► ](javascript:void\(0\)) [ décembre ](https://www.batteman.com/2008/12/) (2)
    * [ ► ](javascript:void\(0\)) [ novembre ](https://www.batteman.com/2008/11/) (5)
    * [ ► ](javascript:void\(0\)) [ septembre ](https://www.batteman.com/2008/09/) (1)
    * [ ► ](javascript:void\(0\)) [ juin ](https://www.batteman.com/2008/06/) (2)
    * [ ► ](javascript:void\(0\)) [ mai ](https://www.batteman.com/2008/05/) (2)
    * [ ► ](javascript:void\(0\)) [ mars ](https://www.batteman.com/2008/03/) (3)
    * [ ► ](javascript:void\(0\)) [ février ](https://www.batteman.com/2008/02/) (4)



## BATTETAGS

[[DIVERS]](https://www.batteman.com/search/label/%5BDIVERS%5D) [[TEST]](https://www.batteman.com/search/label/%5BTEST%5D) [PS3](https://www.batteman.com/search/label/PS3) [[AMIGA]](https://www.batteman.com/search/label/%5BAMIGA%5D) [Amiga](https://www.batteman.com/search/label/Amiga) [PSP](https://www.batteman.com/search/label/PSP) [MorphOS](https://www.batteman.com/search/label/MorphOS) [PC](https://www.batteman.com/search/label/PC) [PSN](https://www.batteman.com/search/label/PSN) [XBox 360](https://www.batteman.com/search/label/XBox%20360) [[LIVRE]](https://www.batteman.com/search/label/%5BLIVRE%5D) [AmigaOS 4](https://www.batteman.com/search/label/AmigaOS%204) [[SONY]](https://www.batteman.com/search/label/%5BSONY%5D) [achat](https://www.batteman.com/search/label/achat) [Playstation Vita](https://www.batteman.com/search/label/Playstation%20Vita) [Jimeo](https://www.batteman.com/search/label/Jimeo) [Pegasos](https://www.batteman.com/search/label/Pegasos) [Uncharted](https://www.batteman.com/search/label/Uncharted) [[MOBILE]](https://www.batteman.com/search/label/%5BMOBILE%5D) [magazine](https://www.batteman.com/search/label/magazine) [Android](https://www.batteman.com/search/label/Android) [David Cage](https://www.batteman.com/search/label/David%20Cage) [Heavy Rain](https://www.batteman.com/search/label/Heavy%20Rain) [Housemarque](https://www.batteman.com/search/label/Housemarque) [Naughty Dog](https://www.batteman.com/search/label/Naughty%20Dog) [PS1](https://www.batteman.com/search/label/PS1) [Quantic Dream](https://www.batteman.com/search/label/Quantic%20Dream) [Super Stardust](https://www.batteman.com/search/label/Super%20Stardust) [Wii](https://www.batteman.com/search/label/Wii) [BD](https://www.batteman.com/search/label/BD) [Cryo](https://www.batteman.com/search/label/Cryo) [Dune](https://www.batteman.com/search/label/Dune) [Flower](https://www.batteman.com/search/label/Flower) [GBA](https://www.batteman.com/search/label/GBA) [Nintendo](https://www.batteman.com/search/label/Nintendo) [PS2](https://www.batteman.com/search/label/PS2) [Resident Evil](https://www.batteman.com/search/label/Resident%20Evil) [ScummVM](https://www.batteman.com/search/label/ScummVM) [Vaches à lait](https://www.batteman.com/search/label/Vaches%20%C3%A0%20lait) [[PERSO]](https://www.batteman.com/search/label/%5BPERSO%5D) [bilan](https://www.batteman.com/search/label/bilan) [bricolage](https://www.batteman.com/search/label/bricolage) [XBox](https://www.batteman.com/search/label/XBox) | 

## WHO IS THE BATTEMAN ?

[ BatteMan ](https://draft.blogger.com/profile/12815652402163334962)
[Afficher mon profil complet](https://draft.blogger.com/profile/12815652402163334962)  
---|---  
  
Inspiré (copié-collé) du thème GameKult que Jimeo m'avait fait. Thème Simple. Fourni par [Blogger](https://draft.blogger.com).
