# Polices de livres (fournies par l'utilisateur)

Ce dossier permet un rendu de l'aperçu de livre **fidèle au jeu**, en utilisant
les polices du jeu.

## Important — droits d'auteur

XIMOD Translator **ne fournit aucune police** ici, volontairement. Les polices
des jeux Bethesda (Skyrim, etc.) sont **protégées par le droit d'auteur de
Bethesda / ZeniMax**. Vous ne devez y placer que des polices que vous êtes
légalement autorisé à utiliser — par exemple celles **extraites de votre propre
copie du jeu**, pour votre usage personnel — et **ne pas les redistribuer**.

## Utilisation

Déposez ici les fichiers de police `.ttf` ou `.otf`. XIMOD les charge au
démarrage et les associe aux « faces » utilisées dans le balisage des livres
Skyrim SE, par correspondance **tolérante** du nom de fichier :

    handwritten.ttf   ← <font face='$HandwrittenFont'>
    SkyrimBooks.ttf   ← texte imprimé par défaut ($SkyrimBooks)
    daedric.ttf       ← <font face='$DaedricFont'>
    dwemer.ttf        ← <font face='$DwemerFont'>
    falmer.ttf        ← <font face='$FalmerFont'>
    dragon.ttf        ← <font face='$DragonFont'>
    … etc.

Le nom exact du fichier importe peu : la correspondance se fait si le nom du
fichier contient le mot-clé de la face (ou l'inverse). Dans la fenêtre
« Aperçu du livre », la case **« Polices du jeu »** active ou désactive ce rendu ;
sans police installée, l'aperçu utilise les polices par défaut (le texte
manuscrit est alors affiché en italique).
