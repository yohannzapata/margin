# Rephrase styles: Qwen3-4B-Instruct-2507-Q4_K_M.gguf

## Standard

- **contr-1** `your going to love it, its really good`
  - `You're gonna love it, it's actually really good.`
- **comma-2** `thanks John I really appreciate it`
  - `Thanks John, I really appreciate it.` / `Thanks, John — I really appreciate it.`
- **period-1** `I finished the report last night I will send it after lunch`
  - `I finished the report last night, gonna send it after lunch.` / `The report was done last night, so I’ll send it after lunch.`
- **long-1** `hey i was thinking about what you said yesterday honestly i dont think thats going to work because we already tried something similar before and it didnt really go well`
  - `Hey, I was thinking about what you said yesterday—honestly, I don't think that's going to work because we already tried something similar before and it didn't really go well.` / `I just thought about your comment from yesterday, and I'm not sure it'll work since we tried something like that before and it didn't turn out well.`
- **long-2** `Hi Maria, thanks for the update. i looked at the numbers and they look good but the budget for march is still missing can you send it when you get a chance`
  - `Hi Maria, thanks for the update. I've checked the numbers and they look good, but the March budget is still missing—can you send it when you get a chance?` / `Hi Maria, thanks for the update. I looked at the numbers and they're in order, but the budget for March is still missing. Could you send it when you have a moment?`
- **run-1** `i went to the store they were out of milk so i got juice instead`
  - `I went to the store and they were out of milk, so I got juice instead.`
- **casual-2** `gonna be late, traffic is crazy rn`
  - (rejected: LostToken("gonna"): "Going to be late—traffic is insane rn.")
- **mixed-2** `the fiesta was so much fun, the lechon was amazing`
  - `The fiesta was a blast and the lechon was absolutely amazing.` / `Fiesta was so much fun, and the lechon was incredible.`
- **ctx-2** `this phone is way better then my old one and the camera is to good`
  - `This phone is way better than my old one and the camera is too good.` / `My new phone is way better than the old one, and the camera is awesome.`
- **ctx-3** `it didnt effect me at all, I'm not sure its working`
  - `It didn't affect me at all, I'm not sure if it's working.` / `The medicine didn't do anything for me, I'm not sure it's effective.`
## Formal

- **contr-1** `your going to love it, its really good`
  - (rejected: TooDifferent: "This will be highly enjoyable and of exceptional quality.")
- **comma-2** `thanks John I really appreciate it`
  - `Thank you, John, for your offer—I truly appreciate it.` / `I am grateful, John, for your support and consideration.`
- **period-1** `I finished the report last night I will send it after lunch`
  - `The report was completed last night and will be sent following lunch hours.` / `I have completed the report last night and will forward it after lunch.`
- **long-1** `hey i was thinking about what you said yesterday honestly i dont think thats going to work because we already tried something similar before and it didnt really go well`
  - `Following up on our conversation from yesterday, I am of the opinion that the suggested plan may not succeed, given that a comparable approach was tried earlier and did not perform well.`
- **long-2** `Hi Maria, thanks for the update. i looked at the numbers and they look good but the budget for march is still missing can you send it when you get a chance`
  - `Hello Maria, thank you for the update. I have reviewed the figures and they appear to be accurate; however, the March budget is still absent. Could you please share it when convenient?` / `Hi Maria, thank you for the update. After reviewing the numbers, they seem favorable, but the March budget is currently missing. Would you be able to send it when you have the opportunity?`
- **run-1** `i went to the store they were out of milk so i got juice instead`
  - `I visited the store and found that milk was unavailable, so I purchased juice as an alternative.` / `Due to the unavailability of milk at the store, I opted for juice instead.`
- **casual-2** `gonna be late, traffic is crazy rn`
  - `I will be arriving later than expected due to significant traffic congestion.` / `I anticipate a delay in my arrival as current traffic conditions are severely disrupted.`
- **mixed-2** `the fiesta was so much fun, the lechon was amazing`
  - `The fiesta was thoroughly enjoyable, and the lechon was exceptionally delicious.` / `The fiesta proved to be a delightful experience, with the lechon standing out as particularly excellent.`
- **ctx-2** `this phone is way better then my old one and the camera is to good`
  - `This phone is significantly better than my previous one, and the camera performance is exceptional.` / `The current phone offers substantial improvements over the earlier model, with particularly outstanding camera capabilities.`
- **ctx-3** `it didnt effect me at all, I'm not sure its working`
  - `The medication did not produce any noticeable effect on me, and I am uncertain whether it is effective.`
## Simple

- **contr-1** `your going to love it, its really good`
  - `You'll love it. It's really good.` / `It's really good. You'll love it.`
- **comma-2** `thanks John I really appreciate it`
  - `Thanks John, I really appreciate it.` / `Thanks, John. I really appreciate it too.`
- **period-1** `I finished the report last night I will send it after lunch`
  - `I finished the report last night. I'll send it after lunch.` / `The report is done from last night. I’ll send it after lunch.`
- **long-1** `hey i was thinking about what you said yesterday honestly i dont think thats going to work because we already tried something similar before and it didnt really go well`
  - `Hey, I was thinking about what you said yesterday. I don't think that will work. We tried something like it before, and it didn't go well.` / `I'm thinking about your suggestion from yesterday. I don't believe it will work. We already tried something similar, and it didn't turn out well.`
- **long-2** `Hi Maria, thanks for the update. i looked at the numbers and they look good but the budget for march is still missing can you send it when you get a chance`
  - `Hi Maria, thanks for the update. I checked the numbers and they look okay. The March budget is still missing. Can you send it when you have a chance?`
- **run-1** `i went to the store they were out of milk so i got juice instead`
  - (rejected: MeaningFlip("negation"): "I went to the store and found no milk. I got juice instead.")
- **casual-2** `gonna be late, traffic is crazy rn`
  - `I'll be late, traffic is really bad right now.` / `Going to be late, traffic is terrible right now.`
- **mixed-2** `the fiesta was so much fun, the lechon was amazing`
  - `The fiesta was really fun. The lechon was delicious.` / `Fiesta was awesome. Lechon was great.`
- **ctx-2** `this phone is way better then my old one and the camera is to good`
  - `This phone is much better than my old one and the camera is great.` / `The new phone is better than the old one and the camera works really well.`
- **ctx-3** `it didnt effect me at all, I'm not sure its working`
  - `The medicine didn't do anything for me. I'm not sure it's working.` / `It had no effect on me at all. I don't think it's working.`
## Creative

- **contr-1** `your going to love it, its really good`
  - (rejected: TooDifferent: "This one's a winner, trust me, it’s truly fantastic.")
- **comma-2** `thanks John I really appreciate it`
  - `Hey John, thanks a ton — I truly mean it!` / `John, your help means a lot — really appreciate it!`
- **period-1** `I finished the report last night I will send it after lunch`
  - `I wrapped up the report last night — sending it right after lunch.` / `The report was all done last night, and I’ll hit send after lunch.`
- **long-1** `hey i was thinking about what you said yesterday honestly i dont think thats going to work because we already tried something similar before and it didnt really go well`
  - `Hey, I was thinking about what you said yesterday—frankly, I don’t see that working, since we’ve already tried something like it before and it didn’t really pan out.` / `I’ve been mulling over your point from yesterday, and honestly, I’m not convinced it’ll work—same setup, same outcome, and it failed before.`
- **long-2** `Hi Maria, thanks for the update. i looked at the numbers and they look good but the budget for march is still missing can you send it when you get a chance`
  - `Hi Maria, thanks for the update — I’ve gone over the numbers and they look solid, but the March budget’s still missing. Could you send it when you’ve got a moment?` / `Hey Maria, thanks for sharing the update. I checked the numbers and they’re looking good, though the March budget’s still missing. Would you be able to send it when you get a chance?`
- **run-1** `i went to the store they were out of milk so i got juice instead`
  - `I headed to the store only to find milk was sold out, so I grabbed juice instead.` / `Stopped by the store and discovered milk was gone—ended up with juice instead.`
- **casual-2** `gonna be late, traffic is crazy rn`
  - (rejected: LostToken("gonna"): "Late tonight, traffic’s chaos in full swing, right now.")
- **mixed-2** `the fiesta was so much fun, the lechon was amazing`
  - `The fiesta was a blast, and the lechon was absolutely incredible.` / `Fiesta was pure joy, and that lechon? A culinary masterpiece.`
- **ctx-2** `this phone is way better then my old one and the camera is to good`
  - `This phone blows my old one out of the water, especially the camera—it's absolutely stunning.` / `The new phone is a game-changer compared to the old one, and the camera? Pure magic.`
- **ctx-3** `it didnt effect me at all, I'm not sure its working`
  - `It didn't affect me one bit—I'm still not seeing any results.`
## Academic

- **contr-1** `your going to love it, its really good`
  - (rejected: TooDifferent: "The experience is anticipated to be favorable, given its outstanding merits.")
- **comma-2** `thanks John I really appreciate it`
  - `Thank you, John, for your support; it is greatly appreciated.`
- **period-1** `I finished the report last night I will send it after lunch`
  - `The report was completed last night; it will be transmitted following the noon meal.` / `Completion of the report occurred last night, and it will be submitted after the afternoon meal.`
- **long-1** `hey i was thinking about what you said yesterday honestly i dont think thats going to work because we already tried something similar before and it didnt really go well`
  - (rejected: TooDifferent: "A review of the suggestion made yesterday indicates that the proposed course of action is not viable, given that a comparable method was previously implemented and failed to achieve satisfactory outcomes.")
- **long-2** `Hi Maria, thanks for the update. i looked at the numbers and they look good but the budget for march is still missing can you send it when you get a chance`
  - `Dear Maria, thank you for the update. Upon reviewing the figures, the data appears satisfactory; however, the March budget remains unavailable. Could you please provide it at your earliest convenience?` / `Hello Maria, I appreciate the information provided. After examining the figures, the results are favorable, yet the budget for March is currently absent. Kindly share it when possible.`
- **run-1** `i went to the store they were out of milk so i got juice instead`
  - `I visited the store, but milk was unavailable; as a result, I purchased juice instead.`
- **casual-2** `gonna be late, traffic is crazy rn`
  - `I will be arriving late due to severe traffic conditions currently affecting the area.` / `There is significant traffic congestion currently in effect, which will result in my delayed arrival.`
- **mixed-2** `the fiesta was so much fun, the lechon was amazing`
  - `The fiesta proved to be an enjoyable experience, and the lechon was exceptionally outstanding.` / `The event was highly entertaining, with the lechon standing out as a particularly exceptional feature.`
- **ctx-2** `this phone is way better then my old one and the camera is to good`
  - `This telephone demonstrates significant improvements over the previous model, particularly in camera performance, which is notably superior.`
- **ctx-3** `it didnt effect me at all, I'm not sure its working`
  - `The medication did not produce any observable effect on me; I am therefore uncertain regarding its efficacy.`

**Standard: 9/10 usable, median 1770 ms; Formal: 9/10 usable, median 1919 ms; Simple: 9/10 usable, median 1785 ms; Creative: 8/10 usable, median 1920 ms; Academic: 8/10 usable, median 2066 ms**
