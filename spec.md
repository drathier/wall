

lysator 40 är 6*17 plattor
varje platta är 15*15, nej 30*30





27*12 plattor hela väggen

27*12=324 rutor
40 röster per år

3x3 första året
2x2 år 2

27*12*30*30/5000*69=4000sek


- inventory management? exakt antal pärlor
- startup: bitcoin-style halvering i antal plattor man röstar om? eller 4st i början, sen ner till 1st över tid?
- rösta på hela pärlplattor
- rösta på vilken ruta man ska rösta om nästa vecka
- en röst per (medlemskap, förening)
-

- ladda upp bilder som är rätt antal pixlar, och enbart godkända färgkoder
- parliamentet har röstpauser under sommar och jul
- spara ner röstvinster
-

färger:
- röd, blå, grön, gul, svart, vit
- Vit, gul, orange, röd, lila, rosa, blå, ljusgrön, brun & svart



Vi bygger en vägg med pärlplattor. Vi vill rösta på vilken platta som ska sättas upp varje vecka. Väggen är 27 plattor bred * 12 plattor hög med 30x30 pärlor per platta. Vi vill visa hur väggen ser ut so far, samt låta folk ladda upp bmp-filer med godkända färger som förslag på vad som ska bytas ut härnäst. Varje vecka röstar man också på vilken koordinat (x+ höger, y+ ner) som ska bytas ut nästa vecka. I början är hela väggen vit.

Skriv en webbserver i rust där folk kan rösta. Vi har en db på vilka som får rösta; det lägger vi till stöd för senare. For now får alla rösta hur många gånger man vill. 

First off, stöd för att ladda upp bmp, visa uppladdade, låt folk rösta, debug-knapp som säger "nu är det en ny vecka", db som sparar alla röstresultat so far, och visa hur väggen ser ut so far.

1. ladda upp bilder att rösta mellan
2. röstning + vem som får rösta
3. db för att spara state so far
4. visualisering av hur världen ser ut idag, samt med respektive ändring


